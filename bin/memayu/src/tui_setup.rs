//! In-process first-run setup wizard for the TUI.
//!
//! When the TUI starts on a fresh instance there is no admin account yet, so
//! instead of telling the user to start `memayu serve` + open a browser, we
//! walk them through creating the admin account right here. The account is
//! created with the exact same logic as the web `POST /api/auth/setup` flow
//! ([`memayu_identity::create_admin_account`]), so a terminal-created admin is
//! byte-for-byte equivalent to one created over HTTP (#32).
//!
//! The wizard owns its own ratatui terminal session and returns the created
//! admin account_id on success, or `None` if the user cancels / setup fails.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use memayu_config::Config;
use memayu_identity::IdentityError;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::time::Duration;

enum Field {
    Email,
    Password,
    Confirm,
}

enum StepResult {
    Continue,
    Done(String),
    Cancel,
}

struct SetupWizard<'a> {
    config: &'a Config,
    email: String,
    password: String,
    confirm: String,
    field: Field,
    error: Option<String>,
}

/// Run the first-run setup wizard. Returns the created admin account_id, or
/// `None` if setup was cancelled or failed.
pub async fn run_setup(config: &Config) -> Option<String> {
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, config).await;
    ratatui::restore();
    result
}

async fn run_loop(terminal: &mut ratatui::DefaultTerminal, config: &Config) -> Option<String> {
    let mut wizard = SetupWizard {
        config,
        email: String::new(),
        password: String::new(),
        confirm: String::new(),
        field: Field::Email,
        error: None,
    };

    loop {
        terminal.draw(|f| draw(f, &wizard)).ok()?;
        if !event::poll(Duration::from_millis(100)).ok()? {
            continue;
        }
        match event::read() {
            Ok(Event::Key(key)) => match handle_key(key, &mut wizard).await {
                StepResult::Continue => {}
                StepResult::Done(id) => return Some(id),
                StepResult::Cancel => return None,
            },
            Ok(Event::Resize(_, _)) => {}
            _ => {}
        }
    }
}

async fn handle_key(key: KeyEvent, wizard: &mut SetupWizard<'_>) -> StepResult {
    // Ctrl-C always cancels.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return StepResult::Cancel;
    }

    match key.code {
        KeyCode::Esc => StepResult::Cancel,
        KeyCode::Char(c) => {
            let field_text = match wizard.field {
                Field::Email => &mut wizard.email,
                Field::Password => &mut wizard.password,
                Field::Confirm => &mut wizard.confirm,
            };
            field_text.push(c);
            StepResult::Continue
        }
        KeyCode::Backspace => {
            let field_text = match wizard.field {
                Field::Email => &mut wizard.email,
                Field::Password => &mut wizard.password,
                Field::Confirm => &mut wizard.confirm,
            };
            field_text.pop();
            StepResult::Continue
        }
        KeyCode::Enter => match wizard.field {
            Field::Email => {
                wizard.field = Field::Password;
                StepResult::Continue
            }
            Field::Password => {
                wizard.field = Field::Confirm;
                StepResult::Continue
            }
            Field::Confirm => submit(wizard).await,
        },
        KeyCode::Tab | KeyCode::Down => {
            wizard.field = next_field(&wizard.field);
            StepResult::Continue
        }
        KeyCode::Up => {
            wizard.field = prev_field(&wizard.field);
            StepResult::Continue
        }
        _ => StepResult::Continue,
    }
}

async fn submit(wizard: &mut SetupWizard<'_>) -> StepResult {
    match memayu_identity::create_admin_account(
        &wizard.config.storage,
        &wizard.email,
        &wizard.password,
        &wizard.confirm,
    )
    .await
    {
        Ok(id) => {
            // Defensive: reassign any legacy placeholder rows to the new admin
            // (a no-op on a genuinely fresh database, #32).
            let _ =
                memayu_identity::backfill_placeholder_memories(&wizard.config.storage, &id).await;
            StepResult::Done(id)
        }
        Err(e) => {
            wizard.error = Some(err_message(&e));
            StepResult::Continue
        }
    }
}

fn next_field(field: &Field) -> Field {
    match field {
        Field::Email => Field::Password,
        Field::Password => Field::Confirm,
        Field::Confirm => Field::Email,
    }
}

fn prev_field(field: &Field) -> Field {
    match field {
        Field::Email => Field::Confirm,
        Field::Password => Field::Email,
        Field::Confirm => Field::Password,
    }
}

fn err_message(err: &IdentityError) -> String {
    match err {
        IdentityError::Validation(msg) => (*msg).to_string(),
        IdentityError::SetupAlreadyCompleted => {
            "an admin account already exists for this instance".to_string()
        }
        IdentityError::Db(msg) => format!("failed to create admin account: {msg}"),
        IdentityError::NoAdminAccount => "no admin account found".to_string(),
    }
}

/// Mask a password for display (one bullet per character).
fn masked(s: &str) -> String {
    "•".repeat(s.chars().count())
}

fn draw(f: &mut Frame, wizard: &SetupWizard<'_>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("memayu — first-run setup");
    let inner = block.inner(f.area());
    f.render_widget(block, f.area());

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(inner);

    let heading = Paragraph::new(
        "No admin account found. Create the admin account to start using memayu from the terminal.",
    )
    .style(Style::default().fg(Color::Cyan));
    f.render_widget(heading, chunks[0]);

    field_row(
        f,
        chunks[1],
        "Email",
        &wizard.email,
        false,
        matches!(wizard.field, Field::Email),
    );
    field_row(
        f,
        chunks[2],
        "Password",
        &masked(&wizard.password),
        true,
        matches!(wizard.field, Field::Password),
    );
    field_row(
        f,
        chunks[3],
        "Confirm password",
        &masked(&wizard.confirm),
        true,
        matches!(wizard.field, Field::Confirm),
    );

    let hint = Paragraph::new(
        "Enter moves to the next field (submit on Confirm). Tab/↑/↓ switch fields. Esc cancels.\n\
         Passwords: at least 8 characters with an uppercase letter, a lowercase letter, and a digit.",
    )
    .style(Style::default().fg(Color::DarkGray));
    f.render_widget(hint, chunks[4]);

    let error_line = if let Some(e) = &wizard.error {
        Line::from(Span::styled(e, Style::default().fg(Color::Red)))
    } else {
        Line::default()
    };
    f.render_widget(Paragraph::new(error_line), chunks[5]);

    // Place the cursor inside the focused field.
    let (rect, text) = match wizard.field {
        Field::Email => (chunks[1], wizard.email.as_str()),
        Field::Password => (chunks[2], wizard.password.as_str()),
        Field::Confirm => (chunks[3], wizard.confirm.as_str()),
    };
    let width = text.chars().count() as u16;
    let x = (rect.x + 1 + width).min(rect.right().saturating_sub(2));
    f.set_cursor_position((x, rect.y + 1));
}

fn field_row(f: &mut Frame, area: Rect, label: &str, value: &str, hidden: bool, focused: bool) {
    let border = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let value_style = if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Yellow)
    };
    let value_text = if hidden && value.is_empty() {
        "(masked — type to enter)".to_string()
    } else {
        value.to_string()
    };
    let paragraph = Paragraph::new(Span::styled(value_text, value_style)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(label),
    );
    f.render_widget(paragraph, area);
}

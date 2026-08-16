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
use memayu_config::{config_path, read_config_file, Config, StorageBackend};
use memayu_identity::IdentityError;
use memayu_llm_client::local_embedder::DEFAULT_MODEL_ID;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::time::Duration;

use memayu_setup::{
    check_device, finalize, fmt_bytes, fmt_cpu, preseed, step_active, step_title, DeviceReport,
    SetupAnswers, SetupResult, SetupStep, DEFAULT_MODEL_INDEX, LOCAL_MODELS, LOCAL_MODEL_NAMES,
    SETUP_STEPS,
};

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

/// A single text input sub-field within a step.
struct TextField {
    label: &'static str,
    hidden: bool,
    getter: fn(&SetupAnswers) -> String,
    setter: fn(&mut SetupAnswers, String),
}

/// A single single-select sub-field within a step.
struct SelectField {
    label: &'static str,
    options: &'static [&'static str],
    getter: fn(&SetupAnswers) -> usize,
    setter: fn(&mut SetupAnswers, usize),
}

enum SubField {
    Text(TextField),
    Select(SelectField),
}

/// Index of the current `embedder_model` within [`LOCAL_MODELS`], defaulting to
/// the default model when it is not one of the offered local models.
fn local_model_get(a: &SetupAnswers) -> usize {
    LOCAL_MODELS
        .iter()
        .position(|m| m.id == a.embedder_model)
        .unwrap_or(DEFAULT_MODEL_INDEX)
}

/// Set `embedder_model` from a selected index into [`LOCAL_MODELS`].
fn local_model_set(a: &mut SetupAnswers, idx: usize) {
    if let Some(m) = LOCAL_MODELS.get(idx) {
        a.embedder_model = m.id.to_string();
    }
}

/// Returns the options for the embedder backend select, honoring whether the
/// device can run local embedding. When unsupported, only `http` is offered.
fn embedder_backend_field(a: &SetupAnswers) -> SelectField {
    if a.device.local_supported {
        SelectField {
            label: "Embedding backend (local = on-device Candle, remote = bring-your-own-key)",
            options: &["local", "remote"],
            getter: |x| if x.embedder_backend == "remote" { 1 } else { 0 },
            setter: |x, v| x.embedder_backend = if v == 1 { "remote" } else { "local" }.to_string(),
        }
    } else {
        // Local is not viable: only remote, forced on commit.
        SelectField {
            label: "Embedding backend (local is not supported on this device, so remote only)",
            options: &["remote"],
            getter: |_| 0,
            setter: |x, _| x.embedder_backend = "remote".to_string(),
        }
    }
}

/// Returns the sub-fields (questions) a step asks, in order. This mirrors the
/// prompts of the CLI presenter so both presenters ask identical questions.
fn step_fields(step: SetupStep, a: &SetupAnswers) -> Vec<SubField> {
    let sb = |x: &SetupAnswers| match x.storage_backend {
        StorageBackend::Libsql => 0,
        StorageBackend::Postgres => 1,
    };
    let sb_set = |x: &mut SetupAnswers, v: usize| {
        x.storage_backend = if v == 1 {
            StorageBackend::Postgres
        } else {
            StorageBackend::Libsql
        }
    };
    let em = |x: &SetupAnswers| if x.extraction_mode == "raw" { 1 } else { 0 };
    let em_set = |x: &mut SetupAnswers, v: usize| {
        x.extraction_mode = if v == 1 { "raw" } else { "llm" }.to_string()
    };

    match step {
        SetupStep::DeviceCheck => Vec::new(),
        SetupStep::StorageBackend => vec![SubField::Select(SelectField {
            label: "Storage backend",
            options: &["libsql", "postgres"],
            getter: sb,
            setter: sb_set,
        })],
        SetupStep::StoragePath => {
            if a.storage_backend == StorageBackend::Libsql {
                vec![SubField::Text(TextField {
                    label: "libsql database path",
                    hidden: false,
                    getter: |x| x.libsql_path.clone(),
                    setter: |x, v| x.libsql_path = v,
                })]
            } else {
                vec![SubField::Text(TextField {
                    label: "Postgres connection URL",
                    hidden: false,
                    getter: |x| x.database_url.clone(),
                    setter: |x, v| x.database_url = v,
                })]
            }
        }
        SetupStep::EmbedderBackend => vec![SubField::Select(embedder_backend_field(a))],
        SetupStep::LocalModel => vec![SubField::Select(SelectField {
            label: "Local embedding model",
            options: LOCAL_MODEL_NAMES,
            getter: local_model_get,
            setter: local_model_set,
        })],
        SetupStep::EmbedderConfig => vec![
            SubField::Text(TextField {
                label: "Embedder base URL",
                hidden: false,
                getter: |x| x.embedder_base_url.clone(),
                setter: |x, v| x.embedder_base_url = v,
            }),
            SubField::Text(TextField {
                label: "Embedder API key (optional)",
                hidden: false,
                getter: |x| x.embedder_api_key.clone(),
                setter: |x, v| x.embedder_api_key = v,
            }),
            SubField::Text(TextField {
                label: "Embedder model",
                hidden: false,
                getter: |x| x.embedder_model.clone(),
                setter: |x, v| x.embedder_model = v,
            }),
        ],
        SetupStep::ExtractionMode => vec![SubField::Select(SelectField {
            label: "Extraction mode",
            options: &["llm", "raw"],
            getter: em,
            setter: em_set,
        })],
        SetupStep::LlmConfig => vec![
            SubField::Text(TextField {
                label: "LLM base URL",
                hidden: false,
                getter: |x| x.llm_base_url.clone(),
                setter: |x, v| x.llm_base_url = v,
            }),
            SubField::Text(TextField {
                label: "LLM API key (optional)",
                hidden: false,
                getter: |x| x.llm_api_key.clone(),
                setter: |x, v| x.llm_api_key = v,
            }),
            SubField::Text(TextField {
                label: "LLM model",
                hidden: false,
                getter: |x| x.llm_model.clone(),
                setter: |x, v| x.llm_model = v,
            }),
        ],
        SetupStep::AdminEmail => vec![SubField::Text(TextField {
            label: "Admin email",
            hidden: false,
            getter: |x| x.admin_email.clone(),
            setter: |x, v| x.admin_email = v,
        })],
        SetupStep::AdminPassword => vec![SubField::Text(TextField {
            label: "Password (min 8 chars, upper+lower+digit)",
            hidden: true,
            getter: |x| x.admin_password.clone(),
            setter: |x, v| x.admin_password = v,
        })],
        SetupStep::AdminConfirm => vec![SubField::Text(TextField {
            label: "Confirm password",
            hidden: true,
            getter: |_| String::new(),
            setter: |_x, _v| {},
        })],
        SetupStep::BindAddr => vec![SubField::Text(TextField {
            label: "Bind address",
            hidden: false,
            getter: |x| x.bind_addr.clone(),
            setter: |x, v| x.bind_addr = v,
        })],
        SetupStep::Port => vec![SubField::Text(TextField {
            label: "Port",
            hidden: false,
            getter: |x| x.port.to_string(),
            setter: |x, v| {
                x.port = v.parse().unwrap_or(18080);
            },
        })],
        SetupStep::ApiKeyLabel => vec![SubField::Text(TextField {
            label: "API key name",
            hidden: false,
            getter: |x| x.api_key_label.clone(),
            setter: |x, v| x.api_key_label = v,
        })],
    }
}

struct FullSetupWizard {
    step: SetupStep,
    progress: (usize, usize), // (current, total) active steps
    fields: Vec<SubField>,
    buffers: Vec<String>,
    field_idx: usize,
    sel: usize,
    error: Option<String>,
    /// Device report shown by the DeviceCheck step (and used to gate local).
    device: DeviceReport,
}

impl FullSetupWizard {
    fn new(step: SetupStep, a: &SetupAnswers) -> Self {
        let fields = step_fields(step, a);
        let buffers = fields
            .iter()
            .map(|f| match f {
                SubField::Text(t) => (t.getter)(a),
                SubField::Select(_) => String::new(),
            })
            .collect();
        let sel = fields
            .iter()
            .find_map(|f| match f {
                SubField::Select(s) => Some((s.getter)(a)),
                _ => None,
            })
            .unwrap_or(0);
        let total = SETUP_STEPS.iter().filter(|&&s| step_active(s, a)).count();
        let current = SETUP_STEPS.iter().position(|&s| s == step).unwrap_or(0) + 1;
        FullSetupWizard {
            step,
            progress: (current, total),
            fields,
            buffers,
            field_idx: 0,
            sel,
            error: None,
            device: a.device.clone(),
        }
    }

    fn focused_is_select(&self) -> bool {
        matches!(self.fields.get(self.field_idx), Some(SubField::Select(_)))
    }

    fn move_cursor(&mut self, delta: i8) {
        if self.focused_is_select() {
            let count = match self.fields[self.field_idx] {
                SubField::Select(ref s) => s.options.len(),
                _ => 1,
            };
            self.sel =
                ((self.sel as i64 + delta as i64).max(0) as usize).min(count.saturating_sub(1));
        } else if self.fields.len() > 1 {
            let next = (self.field_idx as i64 + delta as i64).rem_euclid(self.fields.len() as i64);
            self.field_idx = next as usize;
        }
        self.error = None;
    }

    /// Move to the next sub-field, or commit the step when already on the last
    /// one. Returns `true` when the step was committed (caller should advance
    /// to the next step); `false` when still inside this step.
    fn advance(&mut self, a: &mut SetupAnswers) -> bool {
        if self.field_idx + 1 < self.fields.len() {
            self.field_idx += 1;
            self.error = None;
            false
        } else {
            self.commit(a)
        }
    }

    /// Move to the previous sub-field, or signal the caller to step back a whole
    /// step. Returns `true` when moved within this step; `false` when the caller
    /// should go to the previous step.
    fn go_back(&mut self) -> bool {
        if self.field_idx > 0 {
            self.field_idx -= 1;
            self.error = None;
            true
        } else {
            false
        }
    }

    fn type_char(&mut self, c: char) {
        if let Some(SubField::Text(_)) = self.fields.get(self.field_idx) {
            self.buffers[self.field_idx].push(c);
            self.error = None;
        }
    }

    fn backspace(&mut self) {
        if let Some(SubField::Text(_)) = self.fields.get(self.field_idx) {
            self.buffers[self.field_idx].pop();
            self.error = None;
        }
    }

    /// Commit the current step into `a`. Returns false when validation fails
    /// (the step is kept on screen with an error message).
    fn commit(&mut self, a: &mut SetupAnswers) -> bool {
        if self.step == SetupStep::DeviceCheck {
            // No fields to commit; re-run the probe so the stored report and
            // the on-screen one always agree.
            a.device = check_device();
            self.device = a.device.clone();
            self.error = None;
            return true;
        }
        let mut sel_committed = false;
        for (i, f) in self.fields.iter().enumerate() {
            match f {
                SubField::Text(t) => (t.setter)(a, self.buffers[i].clone()),
                SubField::Select(s) if !sel_committed => {
                    (s.setter)(a, self.sel);
                    sel_committed = true;
                }
                SubField::Select(_) => {}
            }
        }
        // Step-specific validation.
        if self.step == SetupStep::AdminConfirm {
            let confirm = self.buffers.first().cloned().unwrap_or_default();
            if confirm != a.admin_password {
                self.error = Some("passwords do not match".to_string());
                self.field_idx = 0;
                self.buffers[0] = String::new();
                return false;
            }
        }
        // Branch effects after committing a selection.
        if self.step == SetupStep::EmbedderBackend && a.embedder_backend == "local" {
            a.embedder_model = DEFAULT_MODEL_ID.to_string();
        }
        self.error = None;
        true
    }

    fn draw(&self, f: &mut Frame) {
        match self.step {
            SetupStep::DeviceCheck => self.draw_device(f),
            SetupStep::LocalModel => self.draw_local_model(f),
            _ => self.draw_fields(f),
        }
    }

    /// Standard field-oriented rendering for ordinary steps.
    fn draw_fields(&self, f: &mut Frame) {
        let (current, total) = self.progress;
        let block = Block::default().borders(Borders::ALL).title(format!(
            " memayu setup — {} ({}/{}) ",
            step_title(self.step),
            current,
            total
        ));
        let inner = block.inner(f.area());
        f.render_widget(block, f.area());

        let rows = self.fields.len();
        let mut constraints = vec![Constraint::Length(1)];
        for field in &self.fields {
            // A select shows every option plus its two borders; a text field is
            // a fixed 3 rows. Length(3) is too short for a 2-option select, which
            // is why postgres was previously clipped out of storage backend.
            let h = match field {
                SubField::Select(s) => (s.options.len() as u16) + 2,
                SubField::Text(_) => 3,
            };
            constraints.push(Constraint::Length(h));
        }
        constraints.push(Constraint::Length(3));
        constraints.push(Constraint::Length(2));
        constraints.push(Constraint::Min(0));
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        for (i, field) in self.fields.iter().enumerate() {
            let focused = i == self.field_idx;
            let area = chunks[1 + i];
            match field {
                SubField::Text(t) => {
                    let value = &self.buffers[i];
                    let display = if t.hidden {
                        masked(value)
                    } else {
                        value.clone()
                    };
                    field_row(f, area, t.label, &display, t.hidden, focused);
                }
                SubField::Select(s) => {
                    select_row(f, area, s.label, s.options, self.sel, focused);
                }
            }
        }

        let hint = Paragraph::new(
            "Enter/Tab/→: next field (submit on last). ←: previous field/step. ↑/↓: choose option. Esc: cancel.",
        )
        .style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint, chunks[1 + rows]);

        let error_line = if let Some(e) = &self.error {
            Line::from(Span::styled(e, Style::default().fg(Color::Red)))
        } else {
            Line::default()
        };
        f.render_widget(Paragraph::new(error_line), chunks[2 + rows]);

        // Place the cursor inside the focused text field.
        if let Some(SubField::Text(_)) = self.fields.get(self.field_idx) {
            let rect = chunks[1 + self.field_idx];
            let text = &self.buffers[self.field_idx];
            let width = text.chars().count() as u16;
            let x = (rect.x + 1 + width).min(rect.right().saturating_sub(2));
            f.set_cursor_position((x, rect.y + 1));
        }
    }

    /// Full-window report screen for the DeviceCheck step.
    fn draw_device(&self, f: &mut Frame) {
        let (current, total) = self.progress;
        let d = &self.device;
        let block = Block::default().borders(Borders::ALL).title(format!(
            " memayu setup — {} ({}/{}) ",
            step_title(self.step),
            current,
            total
        ));
        let inner = block.inner(f.area());
        f.render_widget(block, f.area());

        let mut lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Device check",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::raw(format!(" OS / arch   {} / {}", d.os, d.arch))),
            Line::from(Span::raw(format!(" CPU         {}", fmt_cpu(d)))),
            Line::from(Span::raw(format!(
                " RAM          {}",
                fmt_bytes(d.ram_bytes)
            ))),
            Line::from(Span::raw(format!(
                " Free disk   {}",
                fmt_bytes(d.free_disk_bytes)
            ))),
        ];
        if d.local_supported {
            lines.push(Line::from(Span::styled(
                " Local embed  supported",
                Style::default().fg(Color::Green),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                " Local embed  NOT supported",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
            for issue in &d.issues {
                lines.push(Line::from(Span::styled(
                    format!("    - {issue}"),
                    Style::default().fg(Color::Red),
                )));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Press Enter to continue",
            Style::default().dim(),
        )));
        f.render_widget(Paragraph::new(lines), inner);
    }

    /// Model picker plus a details pane for the LocalModel step.
    fn draw_local_model(&self, f: &mut Frame) {
        let (current, total) = self.progress;
        let block = Block::default().borders(Borders::ALL).title(format!(
            " memayu setup — {} ({}/{}) ",
            step_title(self.step),
            current,
            total
        ));
        let inner = block.inner(f.area());
        f.render_widget(block, f.area());

        let rows = LOCAL_MODELS.len() as u16;
        let constraints = vec![
            Constraint::Length(1),
            Constraint::Length(rows + 2), // model select
            Constraint::Length(rows + 2), // details pane
            Constraint::Length(3),        // hint
            Constraint::Min(0),
        ];
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        select_row(
            f,
            chunks[1],
            "Local embedding model",
            LOCAL_MODEL_NAMES,
            self.sel,
            true,
        );

        let m = LOCAL_MODELS.get(self.sel).unwrap_or(&LOCAL_MODELS[0]);
        let details = vec![
            Line::from(""),
            Line::from(Span::styled(
                m.name,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::raw(format!(" Dimensions        {}", m.dim))),
            Line::from(Span::raw(format!(
                " Size (fp32/int8) {} MB / {} MB",
                m.fp32_size_mb, m.int8_size_mb
            ))),
            Line::from(Span::raw(format!(" Min RAM           {} MB", m.min_ram_mb))),
            Line::from(Span::raw(format!(" CPU note          {}", m.cpu_notes))),
            Line::from(Span::raw(format!(" Languages         {}", m.langs))),
        ];
        f.render_widget(
            Paragraph::new(details).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Details")
                    .border_style(Style::default().fg(Color::DarkGray)),
            ),
            chunks[2],
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "↑/↓ select · Enter/Tab/→ next · ← back · Esc cancel",
                Style::default().dim(),
            ))),
            chunks[3],
        );
    }
}

fn select_row(
    f: &mut Frame,
    area: Rect,
    label: &str,
    options: &[&str],
    selected: usize,
    focused: bool,
) {
    let border = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    };
    let lines: Vec<Line> = options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            let active = i == selected;
            let prefix = if active { "➤ " } else { "  " };
            let style = if active {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(Span::styled(format!("{prefix}{opt}"), style))
        })
        .collect();
    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(label),
    );
    f.render_widget(paragraph, area);
}

/// Run the full-flow setup wizard in ratatui (`memayu setup --tui`). Returns
/// the setup result on success.
pub async fn run_full_tui_setup() -> Result<SetupResult, Box<dyn std::error::Error>> {
    let existing = read_config_file(&config_path()).ok().flatten();
    let mut answers = preseed(existing.as_ref());
    answers.device = check_device();
    let mut terminal = ratatui::init();
    let result = run_full_loop(&mut terminal, &mut answers).await;
    ratatui::restore();
    result
}

fn ctrl_c(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
}

async fn run_full_loop(
    terminal: &mut ratatui::DefaultTerminal,
    answers: &mut SetupAnswers,
) -> Result<SetupResult, Box<dyn std::error::Error>> {
    // History of SETUP_STEPS indices visited, in order. Pop to go back a step;
    // answers already hold the committed values, so the previous step re-reads
    // them via its getters.
    let mut history: Vec<usize> = Vec::new();

    let mut pos = 0usize;
    while pos < SETUP_STEPS.len() && !step_active(SETUP_STEPS[pos], answers) {
        pos += 1;
    }
    if pos >= SETUP_STEPS.len() {
        return finalize(answers).await;
    }
    history.push(pos);

    let mut wizard = FullSetupWizard::new(SETUP_STEPS[pos], answers);
    loop {
        terminal.draw(|f| wizard.draw(f))?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        match event::read() {
            Ok(Event::Key(key)) => {
                if ctrl_c(key) {
                    return Err("cancelled".into());
                }
                match key.code {
                    KeyCode::Esc => return Err("cancelled".into()),
                    KeyCode::Enter | KeyCode::Tab | KeyCode::Right => {
                        if wizard.advance(answers) {
                            // Find the next active step after the current one.
                            let cur = *history.last().unwrap();
                            let mut next = cur + 1;
                            while next < SETUP_STEPS.len()
                                && !step_active(SETUP_STEPS[next], answers)
                            {
                                next += 1;
                            }
                            if next >= SETUP_STEPS.len() {
                                return finish_screen(terminal, answers).await;
                            }
                            history.push(next);
                            wizard = FullSetupWizard::new(SETUP_STEPS[next], answers);
                        }
                    }
                    KeyCode::Left => {
                        // Move back within the step; if already on the first
                        // field, move back a whole step.
                        if !wizard.go_back() && history.len() > 1 {
                            history.pop();
                            let prev = *history.last().unwrap();
                            wizard = FullSetupWizard::new(SETUP_STEPS[prev], answers);
                        }
                    }
                    KeyCode::Up => wizard.move_cursor(-1),
                    KeyCode::Down => wizard.move_cursor(1),
                    KeyCode::Backspace => wizard.backspace(),
                    KeyCode::Char(c) => wizard.type_char(c),
                    _ => {}
                }
            }
            Ok(Event::Resize(_, _)) => {}
            _ => {}
        }
    }
}

async fn finish_screen(
    terminal: &mut ratatui::DefaultTerminal,
    answers: &SetupAnswers,
) -> Result<SetupResult, Box<dyn std::error::Error>> {
    // Show a busy message while finalize() writes config + creates admin + key.
    terminal
        .draw(|f| {
            let text = Paragraph::new(
                "Writing configuration, creating the admin account, and generating your API key…",
            )
            .style(Style::default().fg(Color::Cyan))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" memayu setup "),
            );
            f.render_widget(text, f.area());
        })
        .ok();

    let result = finalize(answers).await;

    let result = match result {
        Ok(r) => r,
        Err(e) => {
            terminal
                .draw(|f| {
                    let text = Paragraph::new(format!("Setup failed: {e}"))
                        .style(Style::default().fg(Color::Red))
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" memayu setup "),
                        );
                    f.render_widget(text, f.area());
                })
                .ok();
            let _ = wait_for_key();
            return Err(e);
        }
    };

    terminal
        .draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ])
                .split(f.area());
            let title = Paragraph::new("Setup complete").style(Style::default().fg(Color::Green));
            f.render_widget(title, chunks[0]);
            let written = Paragraph::new(format!(
                "Config written to {}",
                result.config_path.display()
            ))
            .style(Style::default().fg(Color::White));
            f.render_widget(written, chunks[1]);
            let key = Paragraph::new(Line::from(vec![
                Span::styled("API key (shown once): ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    &result.api_key,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" copy it now "),
            );
            f.render_widget(key, chunks[2]);
            let hint = Paragraph::new("Press Enter to finish.")
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(hint, chunks[3]);
        })
        .ok();

    let _ = wait_for_key();
    Ok(result)
}

fn wait_for_key() -> Option<()> {
    loop {
        if !event::poll(Duration::from_millis(100)).ok()? {
            continue;
        }
        if let Ok(Event::Key(_)) = event::read() {
            return Some(());
        }
    }
}

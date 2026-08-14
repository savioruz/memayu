//! Ratatui terminal interface for the local memory engine.
//!
//! The TUI talks directly to [`MemoryService`] over the configured storage and
//! providers. The web dashboard and MCP server are optional plugins.

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use memayu_config::Config;
use memayu_core::{Memory, MemoryService};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use std::sync::Arc;

enum Mode {
    Normal,
    Search,
    Add,
    Command,
}

pub struct App {
    service: Arc<MemoryService>,
    account_id: String,
    memories: Vec<Memory>,
    input: String,
    cursor: usize,
    mode: Mode,
    status: String,
    limit: usize,
}

impl App {
    fn new(service: Arc<MemoryService>, account_id: String) -> Self {
        Self {
            service,
            account_id,
            memories: Vec::new(),
            input: String::new(),
            cursor: 0,
            mode: Mode::Normal,
            status: "ready".into(),
            limit: 20,
        }
    }

    fn input_label(&self) -> &'static str {
        match self.mode {
            Mode::Normal => "Command (:h for help)",
            Mode::Search => "Search query (enter to run, esc to cancel)",
            Mode::Add => "New memory content (enter to save, esc to cancel)",
            Mode::Command => "Command (enter to run, esc to cancel)",
        }
    }

    async fn load(&mut self) {
        match self
            .service
            .list_memories(&self.account_id, self.limit)
            .await
        {
            Ok(memories) => {
                self.status = format!("{} memories (limit {})", memories.len(), self.limit);
                self.memories = memories;
            }
            Err(e) => self.status = format!("failed to list memories: {e}"),
        }
    }

    async fn do_search(&mut self) {
        let query = self.input.trim().to_string();
        self.input.clear();
        match self
            .service
            .search_memory(&self.account_id, &query, self.limit)
            .await
        {
            Ok(results) => {
                self.memories = results.into_iter().map(|(m, _)| m).collect();
                self.status = format!("{} results for \"{query}\"", self.memories.len());
            }
            Err(e) => self.status = format!("search failed: {e}"),
        }
    }

    async fn do_add(&mut self) {
        let content = self.input.trim().to_string();
        self.input.clear();
        if content.is_empty() {
            self.status = "nothing to add".into();
            return;
        }
        match self
            .service
            .add_memory(&self.account_id, &content, &Default::default())
            .await
        {
            Ok(_) => self.status = "memory saved".into(),
            Err(e) => self.status = format!("add failed: {e}"),
        }
        self.load().await;
    }

    async fn delete_selected(&mut self) {
        let Some(memory) = self.memories.get(self.cursor).cloned() else {
            self.status = "nothing selected".into();
            return;
        };
        match self.service.delete_memory(&memory.id).await {
            Ok(()) => self.status = format!("deleted {}", memory.id),
            Err(e) => self.status = format!("delete failed: {e}"),
        }
        self.load().await;
    }

    async fn run_command(&mut self) -> bool {
        let cmd = self.input.trim().to_string();
        self.input.clear();
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        match parts.as_slice() {
            [] => {}
            ["q"] | ["quit"] | ["exit"] => return true,
            ["h"] | ["help"] | ["?"] => {
                self.status = HELP.to_string();
            }
            ["list"] => {
                self.load().await;
            }
            ["search"] => {
                self.mode = Mode::Search;
            }
            ["add"] => {
                self.mode = Mode::Add;
            }
            ["delete"] => {
                self.delete_selected().await;
            }
            ["limit", n] => match n.parse::<usize>() {
                Ok(0) => self.status = "limit must be positive".into(),
                Ok(n) => {
                    self.limit = n;
                    self.load().await;
                }
                Err(_) => self.status = format!("invalid limit: {n}"),
            },
            _ => self.status = format!("unknown command: {cmd} (type :h for help)"),
        }
        false
    }

    fn scroll(&mut self, delta: isize) {
        if self.memories.is_empty() {
            return;
        }
        let len = self.memories.len() as isize;
        self.cursor = ((self.cursor as isize + delta).rem_euclid(len)) as usize;
    }
}

const HELP: &str = concat!(
    "q/quit     quit\n",
    "h/help     show this help\n",
    "list       reload memories\n",
    "search     enter search mode\n",
    "add        enter add mode\n",
    "delete     delete the selected memory\n",
    "limit N    set list limit (default 20)\n",
    "↑/↓ or k/j move selection",
);

/// Run the interactive terminal UI.
pub async fn run(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    if config.api_url.is_some() {
        eprintln!("error: the TUI requires a local config — MEMAYU_API_URL is set (cloud mode)");
        std::process::exit(1);
    }

    let (service, dimension) = crate::service::build_service(&config).await?;

    // In-process frontends share the instance's single admin account. Resolve
    // it (backfilling any legacy placeholder rows) or refuse to start so the
    // user isn't silently writing to a "default" memory space (#32).
    let account_id = match memayu_identity::bootstrap(&config.storage).await {
        Ok(id) => id,
        Err(memayu_identity::IdentityError::NoAdminAccount) => {
            // First run: no admin account yet. Run the in-process setup wizard
            // to create one directly — no shelling out to `memayu serve` + the
            // browser, so the TUI still works standalone on a headless VPS (#32).
            match crate::tui_setup::run_setup(&config).await {
                Some(id) => id,
                None => std::process::exit(1),
            }
        }
        Err(e) => return Err(e.into()),
    };

    let mut terminal = ratatui::init();
    terminal.clear()?;

    let mut app = App::new(service, account_id);
    app.status = format!(
        "embedder dimension = {dimension} ({}) | extraction mode = {}",
        config.embedder.model, config.extraction_mode
    );
    app.load().await;

    let result = run_loop(&mut terminal, &mut app).await;

    ratatui::restore();
    result
}

async fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if !event::poll(std::time::Duration::from_millis(100))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) if handle_key(key, app).await => {
                return Ok(());
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

async fn handle_key(key: KeyEvent, app: &mut App) -> bool {
    // Ctrl-C always quits.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return true;
    }

    match app.mode {
        Mode::Normal => match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char(':') => {
                app.input.clear();
                app.mode = Mode::Command;
            }
            KeyCode::Char('j') | KeyCode::Down => app.scroll(1),
            KeyCode::Char('k') | KeyCode::Up => app.scroll(-1),
            KeyCode::Char('d') => app.delete_selected().await,
            KeyCode::Char('s') => app.mode = Mode::Search,
            KeyCode::Char('a') => app.mode = Mode::Add,
            _ => {}
        },
        Mode::Search => match key.code {
            KeyCode::Esc => {
                app.input.clear();
                app.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                app.do_search().await;
                app.mode = Mode::Normal;
            }
            KeyCode::Char(c) => app.input.push(c),
            KeyCode::Backspace => {
                app.input.pop();
            }
            _ => {}
        },
        Mode::Add => match key.code {
            KeyCode::Esc => {
                app.input.clear();
                app.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                app.do_add().await;
                app.mode = Mode::Normal;
            }
            KeyCode::Char(c) => app.input.push(c),
            KeyCode::Backspace => {
                app.input.pop();
            }
            _ => {}
        },
        Mode::Command => match key.code {
            KeyCode::Esc => {
                app.input.clear();
                app.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                if app.run_command().await {
                    return true;
                }
                app.mode = Mode::Normal;
            }
            KeyCode::Char(c) => app.input.push(c),
            KeyCode::Backspace => {
                app.input.pop();
            }
            _ => {}
        },
    }
    false
}

fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(f.area());

    let title = format!("memayu — {} memories", app.memories.len());
    f.render_widget(
        Paragraph::new(title)
            .block(Block::default().borders(Borders::ALL))
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        chunks[0],
    );

    let items: Vec<ListItem> = app
        .memories
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let selected = i == app.cursor;
            let style = if selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let mut line = vec![Span::styled(format!("{:>2} │ ", i + 1), style)];
            let preview: String = m.content.chars().take(80).collect();
            line.push(Span::styled(preview, style));
            if m.content.chars().count() > 80 {
                line.push(Span::styled("…", style));
            }
            ListItem::new(Line::from(line))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Memories (↑/↓ move, d delete, s search, a add, :cmd, q quit)"),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    f.render_widget(list, chunks[1]);

    let status = Paragraph::new(Span::styled(
        app.status.as_str(),
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(status, chunks[2]);

    let input = Paragraph::new(Span::styled(
        app.input.as_str(),
        Style::default().fg(Color::Yellow),
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(app.input_label()),
    );
    f.render_widget(input, chunks[3]);

    if !matches!(app.mode, Mode::Normal) {
        let width = app.input.chars().count() as u16;
        let x = (chunks[3].x + 1 + width).min(chunks[3].right().saturating_sub(2));
        let y = chunks[3].y + 1;
        f.set_cursor_position((x, y));
    }
}

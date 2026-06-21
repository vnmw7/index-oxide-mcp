/*
 * System: Index Oxide MCP
 * Module: Management TUI
 * File URL: index-oxide-mcp/src/manage.rs
 * Purpose: Interactive Terminal User Interface for managing codebase indexes
 */

use crate::clients::InxeQdrantClient;
use crate::clients::embedder::EmbedderClient;
use crate::config::InxeConfig;
use crate::jobs::registry::JobRegistry;
use crate::models::job::{IndexJob, JobStage};
use crate::pipeline::hashing::sanitize_repo_name;
use crate::pipeline::{PipelineOptions, run_pipeline};

use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use std::{io, sync::Arc, time::Duration};
use tokio::sync::{RwLock, mpsc};
use tracing::error;

const TICK_RATE: Duration = Duration::from_millis(100);
const COLLECTION_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const INPUT_BORDER_WIDTH: u16 = 2;
const FALLBACK_CURSOR_COLUMN: u16 = 0;

/// Result of an off-thread collection deletion, reported back to the TUI.
enum DeleteFeedback {
    Deleted(String),
    Failed { collection: String, error: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TuiCommand {
    Help,
    Index(String),
    Refresh,
    Delete(DeleteTarget),
    Confirm,
    Cancel,
    Model(ModelTarget),
    Clear,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeleteTarget {
    Selected,
    Collection(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelTarget {
    Toggle,
    Gemini,
    Ollama,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandParseError {
    message: String,
}

enum CommandOutcome {
    Continue,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbedderProvider {
    Gemini,
    Ollama,
}

pub async fn run_tui(
    config: Arc<InxeConfig>,
    embedder: Arc<RwLock<EmbedderClient>>,
    qdrant: Arc<InxeQdrantClient>,
    jobs: Arc<JobRegistry>,
) -> anyhow::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new(config, embedder, qdrant, jobs);

    // Run the TUI loop
    let res = run_app(&mut terminal, &mut app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err);
    }

    Ok(())
}

struct App {
    config: Arc<InxeConfig>,
    embedder: Arc<RwLock<EmbedderClient>>,
    qdrant: Arc<InxeQdrantClient>,
    jobs: Arc<JobRegistry>,
    input: String,
    input_cursor: usize,
    messages: Vec<String>,
    collections: Vec<String>,
    collections_state: ListState,
    active_model: String,
    last_tick: std::time::Instant,
    confirm_delete: Option<String>,
    delete_tx: mpsc::UnboundedSender<DeleteFeedback>,
    delete_rx: mpsc::UnboundedReceiver<DeleteFeedback>,
}

impl App {
    fn new(
        config: Arc<InxeConfig>,
        embedder: Arc<RwLock<EmbedderClient>>,
        qdrant: Arc<InxeQdrantClient>,
        jobs: Arc<JobRegistry>,
    ) -> App {
        let active_model = config.active_model_name().to_string();
        let (delete_tx, delete_rx) = mpsc::unbounded_channel();
        App {
            config,
            embedder,
            qdrant,
            jobs,
            input: String::new(),
            input_cursor: 0,
            messages: vec![
                "Welcome to Index Oxide MCP Manager.".to_string(),
                "Type /help for commands. Bare paths still start an index job.".to_string(),
            ],
            collections: Vec::new(),
            collections_state: ListState::default(),
            active_model,
            last_tick: std::time::Instant::now(),
            confirm_delete: None,
            delete_tx,
            delete_rx,
        }
    }

    async fn refresh_collections(&mut self) {
        match self.qdrant.list_inxe_collections().await {
            Ok(cols) => self.collections = cols,
            Err(e) => self
                .messages
                .push(format!("Error listing collections: {}", e)),
        }
    }

    async fn start_indexing_path(&mut self, path_str: String) {
        let path_str = path_str.trim().to_string();
        if path_str.is_empty() {
            self.messages.push("Usage: /index <path>".to_string());
            return;
        }

        let repo_path = std::path::Path::new(&path_str);
        if !repo_path.exists() {
            self.messages
                .push(format!("Path does not exist: {}", path_str));
            return;
        }

        let repo_name = sanitize_repo_name(&path_str);
        let job = IndexJob::new(
            uuid::Uuid::new_v4().to_string(),
            path_str.clone(),
            repo_name.clone(),
        );

        self.jobs.register_job(Arc::clone(&job));
        self.messages
            .push(format!("Started indexing: {}", path_str));

        let config = Arc::clone(&self.config);
        let embedder = Arc::clone(&self.embedder);
        let qdrant = Arc::clone(&self.qdrant);
        let pipeline_job = Arc::clone(&job);

        tokio::spawn(async move {
            if let Err(e) = run_pipeline(
                config,
                embedder,
                qdrant,
                pipeline_job,
                PipelineOptions::default(),
            )
            .await
            {
                error!(error = %e, "Pipeline failed");
            }
        });
    }

    fn next_collection(&mut self) {
        let i = match self.collections_state.selected() {
            Some(i) if i >= self.collections.len().saturating_sub(1) => 0,
            Some(i) => i + 1,
            None => 0,
        };
        self.collections_state.select(Some(i));
    }

    fn previous_collection(&mut self) {
        let last = self.collections.len().saturating_sub(1);
        let i = match self.collections_state.selected() {
            Some(0) => last,
            Some(i) => i - 1,
            None => 0,
        };
        self.collections_state.select(Some(i));
    }

    async fn switch_model(&mut self, target: ModelTarget) {
        let desired_provider = {
            let embedder = self.embedder.read().await;
            let current_provider = match &*embedder {
                EmbedderClient::Gemini(_) => EmbedderProvider::Gemini,
                EmbedderClient::Ollama(_) => EmbedderProvider::Ollama,
            };

            match target {
                ModelTarget::Toggle => match current_provider {
                    EmbedderProvider::Gemini => EmbedderProvider::Ollama,
                    EmbedderProvider::Ollama => EmbedderProvider::Gemini,
                },
                ModelTarget::Gemini => {
                    if current_provider == EmbedderProvider::Gemini {
                        self.messages
                            .push(format!("Gemini is already active ({})", self.active_model));
                        return;
                    }
                    EmbedderProvider::Gemini
                }
                ModelTarget::Ollama => {
                    if current_provider == EmbedderProvider::Ollama {
                        self.messages
                            .push(format!("Ollama is already active ({})", self.active_model));
                        return;
                    }
                    EmbedderProvider::Ollama
                }
            }
        };

        let active = self.jobs.list_jobs().iter().any(|j| {
            !matches!(
                j.stage,
                JobStage::Completed | JobStage::Failed | JobStage::Cancelled
            )
        });
        if active {
            self.messages.push(
                "Warning: switching model with active jobs may mix embedding models or break upserts."
                    .into(),
            );
        }

        let mut embedder = self.embedder.write().await;
        match desired_provider {
            EmbedderProvider::Gemini => {
                *embedder = EmbedderClient::Gemini(crate::clients::GeminiClient::new(
                    self.config.gemini.clone(),
                    self.config.embedding.dimensions,
                ));
                self.active_model = self.config.gemini.model.clone();
            }
            EmbedderProvider::Ollama => {
                *embedder = EmbedderClient::Ollama(crate::clients::OllamaClient::new(
                    self.config.ollama.clone(),
                ));
                self.active_model = self.config.ollama.model.clone();
            }
        }
        drop(embedder);
        self.messages
            .push(format!("Switched model to {}", self.active_model));
    }

    fn request_delete(&mut self, target: DeleteTarget) {
        if self.confirm_delete.is_some() {
            self.messages
                .push("A delete confirmation is already pending. Use /confirm or /cancel.".into());
            return;
        }

        let collection = match target {
            DeleteTarget::Selected => {
                let Some(i) = self.collections_state.selected() else {
                    self.messages
                        .push("Select a repository with Up/Down before deleting.".into());
                    return;
                };
                let Some(col) = self.collections.get(i).cloned() else {
                    return;
                };
                col
            }
            DeleteTarget::Collection(name) => {
                let prefixed_name = format!("inxe_{}", name);
                let found = self
                    .collections
                    .iter()
                    .find(|collection| **collection == name || **collection == prefixed_name)
                    .cloned();

                let Some(col) = found else {
                    self.messages
                        .push(format!("Collection not found: {}", name));
                    return;
                };
                col
            }
        };

        self.confirm_delete = Some(collection.clone());
        self.messages.push(format!(
            "Confirm deletion of {} with /confirm, y, or cancel with /cancel.",
            collection
        ));
    }

    async fn perform_delete(&mut self) {
        let Some(col) = self.confirm_delete.take() else {
            self.messages
                .push("No delete confirmation is pending.".into());
            return;
        };

        let repo_name = col.strip_prefix("inxe_").unwrap_or(&col).to_string();

        for job in self.jobs.list_jobs() {
            let is_active = !matches!(
                job.stage,
                JobStage::Completed | JobStage::Failed | JobStage::Cancelled
            );
            if is_active && job.repo_name == repo_name {
                self.jobs.cancel_job(&job.job_id);
                self.messages.push(format!(
                    "Cancelled active job {} ({})",
                    job.job_id, repo_name
                ));
            }
        }

        if let Some(idx) = self.collections.iter().position(|c| *c == col) {
            self.collections.remove(idx);
            self.adjust_selection_after_removal();
        }

        self.messages.push(format!("Deleting {}...", col));

        let qdrant = Arc::clone(&self.qdrant);
        let tx = self.delete_tx.clone();
        tokio::spawn(async move {
            match qdrant.delete_collection_by_name(&col).await {
                Ok(_) => {
                    let _ = tx.send(DeleteFeedback::Deleted(col));
                }
                Err(e) => {
                    let _ = tx.send(DeleteFeedback::Failed {
                        collection: col,
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    fn cancel_delete(&mut self) {
        if let Some(col) = self.confirm_delete.take() {
            self.messages.push(format!("Cancelled deletion of {}", col));
        } else {
            self.messages
                .push("No delete confirmation is pending.".into());
        }
    }

    fn adjust_selection_after_removal(&mut self) {
        let new_len = self.collections.len();
        if new_len == 0 {
            self.collections_state.select(None);
            return;
        }
        let idx = self
            .collections_state
            .selected()
            .unwrap_or(0)
            .min(new_len.saturating_sub(1));
        self.collections_state.select(Some(idx));
    }

    fn selected_collection_label(&self) -> String {
        self.collections_state
            .selected()
            .and_then(|idx| self.collections.get(idx))
            .cloned()
            .unwrap_or_else(|| "none".to_string())
    }

    fn clear_input(&mut self) {
        self.input.clear();
        self.input_cursor = 0;
    }

    fn insert_input_text(&mut self, text: &str) {
        let byte_index = byte_index_for_char(&self.input, self.input_cursor);
        self.input.insert_str(byte_index, text);
        self.input_cursor += text.chars().count();
    }

    fn backspace_input_char(&mut self) {
        if self.input_cursor == 0 {
            return;
        }

        let start = byte_index_for_char(&self.input, self.input_cursor - 1);
        let end = byte_index_for_char(&self.input, self.input_cursor);
        self.input.replace_range(start..end, "");
        self.input_cursor -= 1;
    }

    fn delete_input_char(&mut self) {
        if self.input_cursor >= self.input.chars().count() {
            return;
        }

        let start = byte_index_for_char(&self.input, self.input_cursor);
        let end = byte_index_for_char(&self.input, self.input_cursor + 1);
        self.input.replace_range(start..end, "");
    }

    fn move_input_left(&mut self) {
        self.input_cursor = self.input_cursor.saturating_sub(1);
    }

    fn move_input_right(&mut self) {
        self.input_cursor = (self.input_cursor + 1).min(self.input.chars().count());
    }

    fn move_input_home(&mut self) {
        self.input_cursor = 0;
    }

    fn move_input_end(&mut self) {
        self.input_cursor = self.input.chars().count();
    }

    fn input_view(&self, width: u16) -> (String, u16) {
        if width == 0 {
            return (String::new(), FALLBACK_CURSOR_COLUMN);
        }

        let width = width as usize;
        let input_len = self.input.chars().count();
        if input_len <= width {
            return (self.input.clone(), self.input_cursor as u16);
        }

        let start = if self.input_cursor < width {
            0
        } else {
            self.input_cursor + 1 - width
        };
        let visible = self.input.chars().skip(start).take(width).collect();
        let cursor_column = self.input_cursor.saturating_sub(start).min(width - 1) as u16;

        (visible, cursor_column)
    }

    fn show_help(&mut self) {
        self.messages.extend(
            [
                "Commands: /index <path>, /refresh, /delete [selected|collection]",
                "/confirm, /cancel, /model [gemini|ollama], /clear, /quit",
                "Shortcuts: Up/Down select, Del delete selected, Tab toggles model, Ctrl+Q quits.",
            ]
            .into_iter()
            .map(ToString::to_string),
        );
    }
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> anyhow::Result<()> {
    app.refresh_collections().await;

    loop {
        terminal.draw(|f| ui(f, app))?;

        while let Ok(feedback) = app.delete_rx.try_recv() {
            match feedback {
                DeleteFeedback::Deleted(col) => {
                    app.messages.push(format!("Deleted {}", col));
                }
                DeleteFeedback::Failed { collection, error } => {
                    app.messages
                        .push(format!("Failed to delete {}: {}", collection, error));
                    if !app.collections.contains(&collection) {
                        app.collections.push(collection);
                    }
                }
            }
        }

        if event::poll(TICK_RATE)? {
            match event::read()? {
                Event::Key(key) if key.kind == event::KeyEventKind::Press => {
                    let should_quit = handle_key_press(app, key).await?;
                    if should_quit {
                        return Ok(());
                    }
                }
                Event::Paste(text) => {
                    app.insert_input_text(&text);
                }
                _ => {}
            }
        }

        if app.last_tick.elapsed() >= COLLECTION_REFRESH_INTERVAL {
            app.refresh_collections().await;
            app.last_tick = std::time::Instant::now();
        }
    }
}

async fn handle_key_press(app: &mut App, key: event::KeyEvent) -> anyhow::Result<bool> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(matches!(key.code, KeyCode::Char('q')));
    }

    if app.confirm_delete.is_some()
        && app.input.is_empty()
        && matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'))
    {
        app.perform_delete().await;
        return Ok(false);
    }

    if app.confirm_delete.is_some()
        && app.input.is_empty()
        && matches!(key.code, KeyCode::Char('n') | KeyCode::Char('N'))
    {
        app.cancel_delete();
        return Ok(false);
    }

    match key.code {
        KeyCode::Enter => handle_command_entry(app).await,
        KeyCode::Char(c) => {
            app.insert_input_text(&c.to_string());
            Ok(false)
        }
        KeyCode::Backspace => {
            app.backspace_input_char();
            Ok(false)
        }
        KeyCode::Delete => {
            if app.input.is_empty() {
                app.request_delete(DeleteTarget::Selected);
            } else {
                app.delete_input_char();
            }
            Ok(false)
        }
        KeyCode::Esc => {
            app.clear_input();
            if app.confirm_delete.is_some() {
                app.cancel_delete();
            } else {
                app.collections_state.select(None);
            }
            Ok(false)
        }
        KeyCode::Left => {
            app.move_input_left();
            Ok(false)
        }
        KeyCode::Right => {
            app.move_input_right();
            Ok(false)
        }
        KeyCode::Home => {
            app.move_input_home();
            Ok(false)
        }
        KeyCode::End => {
            app.move_input_end();
            Ok(false)
        }
        KeyCode::Down => {
            app.next_collection();
            Ok(false)
        }
        KeyCode::Up => {
            app.previous_collection();
            Ok(false)
        }
        KeyCode::Tab if app.input.is_empty() => {
            app.switch_model(ModelTarget::Toggle).await;
            Ok(false)
        }
        _ => Ok(false),
    }
}

async fn handle_command_entry(app: &mut App) -> anyhow::Result<bool> {
    let input = app.input.trim().to_string();
    if input.is_empty() {
        return Ok(false);
    }

    match parse_command(&input) {
        Ok(command) => {
            app.clear_input();
            match execute_command(app, command).await? {
                CommandOutcome::Continue => Ok(false),
                CommandOutcome::Quit => Ok(true),
            }
        }
        Err(err) => {
            app.messages.push(err.message);
            Ok(false)
        }
    }
}

async fn execute_command(app: &mut App, command: TuiCommand) -> anyhow::Result<CommandOutcome> {
    match command {
        TuiCommand::Help => app.show_help(),
        TuiCommand::Index(path) => app.start_indexing_path(path).await,
        TuiCommand::Refresh => {
            app.refresh_collections().await;
            app.last_tick = std::time::Instant::now();
            app.messages.push("Collections refreshed.".to_string());
        }
        TuiCommand::Delete(target) => app.request_delete(target),
        TuiCommand::Confirm => app.perform_delete().await,
        TuiCommand::Cancel => {
            if app.confirm_delete.is_some() {
                app.cancel_delete();
            } else {
                app.clear_input();
                app.messages.push("Command input cleared.".to_string());
            }
        }
        TuiCommand::Model(target) => app.switch_model(target).await,
        TuiCommand::Clear => app.messages.clear(),
        TuiCommand::Quit => return Ok(CommandOutcome::Quit),
    }

    Ok(CommandOutcome::Continue)
}

fn parse_command(input: &str) -> Result<TuiCommand, CommandParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(CommandParseError::new(
            "Enter a command or repository path.",
        ));
    }

    if !trimmed.starts_with('/') {
        return Ok(TuiCommand::Index(trimmed.to_string()));
    }

    let command_text = trimmed.trim_start_matches('/');
    let (command_name, args_text) = split_command_name(command_text);
    let command_name = command_name.to_ascii_lowercase();

    match command_name.as_str() {
        "" | "help" | "h" | "?" => {
            reject_args("help", args_text)?;
            Ok(TuiCommand::Help)
        }
        "index" | "i" => Ok(TuiCommand::Index(parse_index_path(args_text)?)),
        "refresh" | "r" => {
            reject_args("refresh", args_text)?;
            Ok(TuiCommand::Refresh)
        }
        "delete" | "del" | "d" => parse_delete_command(args_text),
        "confirm" | "yes" => {
            reject_args("confirm", args_text)?;
            Ok(TuiCommand::Confirm)
        }
        "cancel" | "no" => {
            reject_args("cancel", args_text)?;
            Ok(TuiCommand::Cancel)
        }
        "model" | "m" => parse_model_command(args_text),
        "clear" => {
            reject_args("clear", args_text)?;
            Ok(TuiCommand::Clear)
        }
        "quit" | "exit" | "q" => {
            reject_args("quit", args_text)?;
            Ok(TuiCommand::Quit)
        }
        _ => Err(CommandParseError::new(format!(
            "Unknown command: /{}. Type /help for commands.",
            command_name
        ))),
    }
}

fn split_command_name(command_text: &str) -> (&str, &str) {
    let command_text = command_text.trim_start();
    match command_text.find(char::is_whitespace) {
        Some(index) => (&command_text[..index], command_text[index..].trim()),
        None => (command_text, ""),
    }
}

fn parse_index_path(args_text: &str) -> Result<String, CommandParseError> {
    if args_text.is_empty() {
        return Err(CommandParseError::new("Usage: /index <path>"));
    }

    if args_text.starts_with('"') || args_text.starts_with('\'') {
        let args = parse_shell_args(args_text)?;
        if args.len() != 1 {
            return Err(CommandParseError::new(
                "Usage: /index <path>. Quote paths that contain spaces.",
            ));
        }
        return Ok(args[0].clone());
    }

    if args_text.contains('"') || args_text.contains('\'') {
        return Err(CommandParseError::new(
            "Malformed path quoting. Quote the full path or remove the quote.",
        ));
    }

    Ok(args_text.to_string())
}

fn parse_delete_command(args_text: &str) -> Result<TuiCommand, CommandParseError> {
    if args_text.is_empty() {
        return Ok(TuiCommand::Delete(DeleteTarget::Selected));
    }

    let args = parse_shell_args(args_text)?;
    if args.len() != 1 {
        return Err(CommandParseError::new(
            "Usage: /delete [selected|collection]",
        ));
    }

    if args[0].eq_ignore_ascii_case("selected") {
        Ok(TuiCommand::Delete(DeleteTarget::Selected))
    } else {
        Ok(TuiCommand::Delete(DeleteTarget::Collection(
            args[0].clone(),
        )))
    }
}

fn parse_model_command(args_text: &str) -> Result<TuiCommand, CommandParseError> {
    if args_text.is_empty() {
        return Ok(TuiCommand::Model(ModelTarget::Toggle));
    }

    let args = parse_shell_args(args_text)?;
    if args.len() != 1 {
        return Err(CommandParseError::new("Usage: /model [gemini|ollama]"));
    }

    match args[0].to_ascii_lowercase().as_str() {
        "gemini" => Ok(TuiCommand::Model(ModelTarget::Gemini)),
        "ollama" => Ok(TuiCommand::Model(ModelTarget::Ollama)),
        _ => Err(CommandParseError::new(
            "Unknown model. Use /model gemini or /model ollama.",
        )),
    }
}

fn reject_args(command_name: &str, args_text: &str) -> Result<(), CommandParseError> {
    if args_text.is_empty() {
        Ok(())
    } else {
        Err(CommandParseError::new(format!(
            "/{} does not accept arguments.",
            command_name
        )))
    }
}

fn parse_shell_args(args_text: &str) -> Result<Vec<String>, CommandParseError> {
    shlex::split(args_text).ok_or_else(|| {
        CommandParseError::new("Malformed command quoting. Check for an unmatched quote.")
    })
}

impl CommandParseError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

fn byte_index_for_char(text: &str, cursor: usize) -> usize {
    text.char_indices()
        .nth(cursor)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(7),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(f.size());

    render_status(f, app, chunks[0]);

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(chunks[1]);

    let collections: Vec<ListItem> = app
        .collections
        .iter()
        .map(|i| ListItem::new(i.as_str()))
        .collect();
    let collections_list = List::new(collections)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Indexed Repositories"),
        )
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        )
        .highlight_symbol(">> ");
    f.render_stateful_widget(collections_list, main_chunks[0], &mut app.collections_state);

    let active_jobs = app.jobs.list_jobs();
    let jobs_items: Vec<ListItem> = active_jobs
        .iter()
        .map(|j| {
            ListItem::new(format!(
                "{} [{:?}] - D:{} P:{} E:{} I:{}",
                j.repo_name,
                j.stage,
                j.counters.discovered,
                j.counters.chunked,
                j.counters.embedded,
                j.counters.indexed
            ))
        })
        .collect();
    let jobs_list =
        List::new(jobs_items).block(Block::default().borders(Borders::ALL).title("Index Jobs"));
    f.render_widget(jobs_list, main_chunks[1]);

    render_logs(f, app, chunks[2]);
    render_prompt(f, app, chunks[3]);
}

fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let pending_delete = app
        .confirm_delete
        .as_ref()
        .map(|collection| format!(" | Pending delete: {}", collection))
        .unwrap_or_default();

    let status = format!(
        "Active model: {} | Selected: {} | Collections: {} | Jobs: {} | Refresh: {}s{}",
        app.active_model,
        app.selected_collection_label(),
        app.collections.len(),
        app.jobs.list_jobs().len(),
        app.last_tick.elapsed().as_secs(),
        pending_delete
    );

    let status_panel = Paragraph::new(status)
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL).title("Manager"));
    f.render_widget(status_panel, area);
}

fn render_logs(f: &mut Frame, app: &App, area: Rect) {
    let visible_rows = area.height.saturating_sub(INPUT_BORDER_WIDTH) as usize;
    let messages: Vec<ListItem> = app
        .messages
        .iter()
        .rev()
        .take(visible_rows)
        .map(|m| ListItem::new(m.as_str()))
        .collect();
    let messages_list =
        List::new(messages).block(Block::default().borders(Borders::ALL).title("Logs"));
    f.render_widget(messages_list, area);
}

fn render_prompt(f: &mut Frame, app: &App, area: Rect) {
    let inner_width = area.width.saturating_sub(INPUT_BORDER_WIDTH);
    let (visible_input, cursor_column) = app.input_view(inner_width);
    let title = match app.confirm_delete {
        Some(_) => "Command | confirm pending: y, n/Esc, /confirm, /cancel",
        None => "Command | /help for commands | bare path indexes",
    };
    let input = Paragraph::new(visible_input)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(input, area);

    if inner_width > 0 {
        f.set_cursor(area.x + 1 + cursor_column, area.y + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_index_command_with_windows_path() {
        assert_eq!(
            parse_command(r"/index D:\projects\index-oxide-mcp").unwrap(),
            TuiCommand::Index(r"D:\projects\index-oxide-mcp".to_string())
        );
    }

    #[test]
    fn parses_index_command_with_quoted_windows_path() {
        assert_eq!(
            parse_command(r#"/index "D:\path with spaces\repo""#).unwrap(),
            TuiCommand::Index(r"D:\path with spaces\repo".to_string())
        );
    }

    #[test]
    fn parses_bare_path_as_index_command() {
        assert_eq!(
            parse_command(r"D:\projects\index-oxide-mcp").unwrap(),
            TuiCommand::Index(r"D:\projects\index-oxide-mcp".to_string())
        );
    }

    #[test]
    fn parses_delete_commands() {
        assert_eq!(
            parse_command("/delete").unwrap(),
            TuiCommand::Delete(DeleteTarget::Selected)
        );
        assert_eq!(
            parse_command("/delete selected").unwrap(),
            TuiCommand::Delete(DeleteTarget::Selected)
        );
        assert_eq!(
            parse_command("/delete inxe_repo").unwrap(),
            TuiCommand::Delete(DeleteTarget::Collection("inxe_repo".to_string()))
        );
    }

    #[test]
    fn parses_model_commands() {
        assert_eq!(
            parse_command("/model").unwrap(),
            TuiCommand::Model(ModelTarget::Toggle)
        );
        assert_eq!(
            parse_command("/model gemini").unwrap(),
            TuiCommand::Model(ModelTarget::Gemini)
        );
        assert_eq!(
            parse_command("/model ollama").unwrap(),
            TuiCommand::Model(ModelTarget::Ollama)
        );
    }

    #[test]
    fn rejects_unknown_command() {
        let err = parse_command("/unknown").unwrap_err();
        assert!(err.message.contains("Unknown command"));
    }

    #[test]
    fn rejects_unmatched_quote() {
        let err = parse_command(r#"/index "D:\path with spaces\repo"#).unwrap_err();
        assert!(err.message.contains("Malformed command quoting"));
    }
}

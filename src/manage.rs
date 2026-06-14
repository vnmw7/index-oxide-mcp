/*
 * System: Index Oxide MCP
 * Module: Management TUI
 * File URL: index-oxide-mcp/src/manage.rs
 * Purpose: Interactive Terminal User Interface for managing codebase indexes
 */

use crate::config::InxeConfig;
use crate::clients::embedder::EmbedderClient;
use crate::jobs::registry::JobRegistry;
use crate::pipeline::{run_pipeline, PipelineOptions};
use crate::models::job::IndexJob;
use crate::clients::InxeQdrantClient;
use crate::pipeline::hashing::sanitize_repo_name;

use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::{io, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tracing::error;

pub async fn run_tui(
    config: Arc<InxeConfig>,
    embedder: Arc<RwLock<EmbedderClient>>,
    qdrant: Arc<InxeQdrantClient>,
    jobs: Arc<JobRegistry>,
) -> anyhow::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
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
    messages: Vec<String>,
    collections: Vec<String>,
    last_tick: std::time::Instant,
}

impl App {
    fn new(
        config: Arc<InxeConfig>,
        embedder: Arc<RwLock<EmbedderClient>>,
        qdrant: Arc<InxeQdrantClient>,
        jobs: Arc<JobRegistry>,
    ) -> App {
        App {
            config,
            embedder,
            qdrant,
            jobs,
            input: String::new(),
            messages: vec!["Welcome to Index Oxide MCP Manager. Press Ctrl+Q to quit.".to_string()],
            collections: Vec::new(),
            last_tick: std::time::Instant::now(),
        }
    }

    async fn refresh_collections(&mut self) {
        match self.qdrant.list_inxe_collections().await {
            Ok(cols) => self.collections = cols,
            Err(e) => self.messages.push(format!("Error listing collections: {}", e)),
        }
    }

    async fn start_indexing(&mut self) {
        let path_str = self.input.trim().to_string();
        if path_str.is_empty() {
            return;
        }

        let repo_path = std::path::Path::new(&path_str);
        if !repo_path.exists() {
            self.messages.push(format!("Path does not exist: {}", path_str));
            return;
        }

        let repo_name = sanitize_repo_name(&path_str);
        let job = IndexJob::new(
            uuid::Uuid::new_v4().to_string(),
            path_str.clone(),
            repo_name.clone(),
        );

        self.jobs.register_job(Arc::clone(&job));
        self.messages.push(format!("Started indexing: {}", path_str));
        self.input.clear();

        let config = Arc::clone(&self.config);
        let embedder = Arc::clone(&self.embedder);
        let qdrant = Arc::clone(&self.qdrant);
        let pipeline_job = Arc::clone(&job);

        tokio::spawn(async move {
            if let Err(e) = run_pipeline(config, embedder, qdrant, pipeline_job, PipelineOptions::default()).await {
                error!(error = %e, "Pipeline failed");
            }
        });
    }
}

async fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> anyhow::Result<()> {
    app.refresh_collections().await;

    loop {
        terminal.draw(|f| ui(f, app))?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == event::KeyEventKind::Press {
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            match key.code {
                                KeyCode::Char('q') => return Ok(()),
                                _ => {}
                            }
                        } else {
                            match key.code {
                                KeyCode::Enter => {
                                    app.start_indexing().await;
                                }
                                KeyCode::Char(c) => {
                                    app.input.push(c);
                                }
                                KeyCode::Backspace => {
                                    app.input.pop();
                                }
                                KeyCode::Esc => {
                                    app.input.clear();
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Event::Paste(text) => {
                    app.input.push_str(&text);
                }
                _ => {}
            }
        }

        if app.last_tick.elapsed() >= Duration::from_secs(5) {
            app.refresh_collections().await;
            app.last_tick = std::time::Instant::now();
        }
    }
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(5),
            ]
            .as_ref(),
        )
        .split(f.size());

    // Input area
    let input = Paragraph::new(app.input.as_str())
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::ALL).title("Index New Directory (Enter Path)"));
    f.render_widget(input, chunks[0]);

    // Main area: Collections and Jobs
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
        .block(Block::default().borders(Borders::ALL).title("Indexed Repositories"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol(">>");
    f.render_widget(collections_list, main_chunks[0]);

    // Active Jobs status
    let active_jobs = app.jobs.list_jobs();
    let jobs_items: Vec<ListItem> = active_jobs
        .iter()
        .map(|j| {
            ListItem::new(format!(
                "{} [{:?}] - D:{} P:{} E:{} I:{}",
                j.repo_name, j.stage, j.counters.discovered, j.counters.chunked, j.counters.embedded, j.counters.indexed
            ))
        })
        .collect();
    let jobs_list = List::new(jobs_items)
        .block(Block::default().borders(Borders::ALL).title("Index Jobs Status"));
    f.render_widget(jobs_list, main_chunks[1]);

    // Messages / Log area
    let messages: Vec<ListItem> = app
        .messages
        .iter()
        .rev()
        .take(4)
        .map(|m| ListItem::new(m.as_str()))
        .collect();
    let messages_list = List::new(messages)
        .block(Block::default().borders(Borders::ALL).title("Logs"));
    f.render_widget(messages_list, chunks[2]);
}

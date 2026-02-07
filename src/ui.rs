use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    style::Stylize,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
    Frame, Terminal,
};
use std::collections::VecDeque;
use std::io;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing_subscriber::fmt::MakeWriter;

// Mensajes que enviamos de la TUI al worker
pub enum AppEvent {
    Input(String),
}

// Mensajes que recibimos del worker en la TUI
pub enum WorkerEvent {
    Response(String),
    Log(String),
}

#[derive(Clone, Debug)]
pub enum MessageAuthor {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub author: MessageAuthor,
    pub text: String,
}

// Estructura para capturar logs en memoria para la TUI
#[derive(Clone)]
pub struct TuiLogger {
    logs: Arc<Mutex<VecDeque<String>>>,
}

impl TuiLogger {
    pub fn new() -> Self {
        Self {
            logs: Arc::new(Mutex::new(VecDeque::with_capacity(100))),
        }
    }

    pub fn get_logs(&self) -> Vec<String> {
        let logs = self.logs.lock().unwrap();
        logs.iter().cloned().collect()
    }
}

impl std::io::Write for TuiLogger {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let log_line = String::from_utf8_lossy(buf).to_string();
        let mut logs = self.logs.lock().unwrap();
        if logs.len() >= 100 {
            logs.pop_front();
        }
        logs.push_back(log_line.trim().to_string());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// Para usar con tracing_subscriber
impl<'a> MakeWriter<'a> for TuiLogger {
    type Writer = TuiLogger;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

pub struct App {
    pub input: String,
    pub messages: Vec<ChatMessage>,
    pub logger: TuiLogger,
    pub tx: Sender<AppEvent>,
    pub rx: Receiver<WorkerEvent>,
    pub scroll: u16,
    pub content_height: u16,
    pub input_mode: InputMode,
    pub is_loading: bool,
}

#[derive(PartialEq)]
pub enum InputMode {
    Normal,
    Editing,
}

impl App {
    pub fn new(logger: TuiLogger, tx: Sender<AppEvent>, rx: Receiver<WorkerEvent>) -> App {
        App {
            input: String::new(),
            messages: vec![ChatMessage {
                author: MessageAuthor::Assistant,
                text: "🚀 TUI initialized. Press 'i' to type, 'Esc' to scroll chat.".to_string(),
            }],
            logger,
            tx,
            rx,
            scroll: 0,
            content_height: 0,
            input_mode: InputMode::Editing, // Empezar en modo edición por comodidad
            is_loading: false,
        }
    }
}

pub fn run_tui(mut app: App) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        // Procesar respuestas del worker
        while let Ok(event) = app.rx.try_recv() {
            match event {
                WorkerEvent::Response(res) => {
                    app.is_loading = false;
                    app.messages.push(ChatMessage {
                        author: MessageAuthor::Assistant,
                        text: res,
                    });
                }
                WorkerEvent::Log(_msg) => {}
            }
        }

        terminal.draw(|f: &mut Frame| {
            let area = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(
                    [
                        Constraint::Min(10),   // Main Chat
                        Constraint::Length(5), // Logs
                        Constraint::Length(3), // Input
                    ]
                    .as_ref(),
                )
                .split(area);

            // 1. Chat Area
            let mut chat_text = Vec::new();
            for msg in &app.messages {
                match msg.author {
                    MessageAuthor::User => {
                        chat_text.push(Line::from(vec![
                            Span::raw("💬 "),
                            Span::styled(&msg.text, Style::default().fg(Color::Yellow)),
                        ]));
                    }
                    MessageAuthor::Assistant => {
                        let mut in_tool_result = false;
                        for line in msg.text.lines() {
                            if line.starts_with("[RCLAW_USE_TOOL]") {
                                in_tool_result = false;
                                let content = line.trim_start_matches("[RCLAW_USE_TOOL]");
                                chat_text.push(Line::from(Span::styled(
                                    format!("  🔨 {}", content),
                                    Style::default().fg(Color::Magenta),
                                )));
                            } else if line.starts_with("[RCLAW_TOOL_RESULT]") {
                                in_tool_result = true;
                                let mut content = line.trim_start_matches("[RCLAW_TOOL_RESULT]").to_string();
                                
                                // Si la línea contiene el marcador de fin, lo quitamos y cerramos el estado gris
                                if content.contains("[RCLAW_END_RESULT]") {
                                    content = content.replace("[RCLAW_END_RESULT]", "");
                                    in_tool_result = false;
                                }

                                if !content.is_empty() {
                                    chat_text.push(Line::from(Span::styled(
                                        format!("  {}", content),
                                        Style::default().fg(Color::DarkGray),
                                    )));
                                }
                            } else {
                                if line.trim().is_empty() {
                                    chat_text.push(Line::from(""));
                                    continue;
                                }

                                let style = if in_tool_result {
                                    Style::default().fg(Color::DarkGray)
                                } else {
                                    Style::default().fg(Color::White)
                                };

                                chat_text.push(Line::from(Span::styled(
                                    format!("  {}", line),
                                    style,
                                )));
                                
                                // Si no estamos en un bloque de resultado explícito, cualquier línea de texto es normal
                                // Nota: in_tool_result solo se mantiene true si no encontramos el marcador de fin
                            }
                        }
                    }
                }
                chat_text.push(Line::from(""));
            }

            if app.is_loading {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO)
                    .as_millis();
                let dot_count = (now / 500) % 3 + 1;
                let dots = ".".repeat(dot_count as usize);
                chat_text.push(Line::from(Span::styled(
                    format!("  {}", dots),
                    Style::default().fg(Color::DarkGray),
                )));
            }

            // Cálculo de scroll
            let line_count = chat_text.len() as u16;
            let viewport_height = chunks[0].height.saturating_sub(2);

            // Auto-scroll si el contenido crece y estamos cerca del final
            if line_count > viewport_height {
                if app.scroll >= app.content_height.saturating_sub(viewport_height) {
                    app.scroll = line_count.saturating_sub(viewport_height);
                }
            } else {
                app.scroll = 0;
            }
            app.content_height = line_count;

            let chat_title = if app.input_mode == InputMode::Normal {
                " Rclaw Chat (SCROLL MODE - Up/Down to navigate, 'i' to type) "
            } else {
                " Rclaw Chat "
            };

            let chat_paragraph = Paragraph::new(chat_text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(chat_title)
                        .padding(Padding::uniform(1))
                        .border_style(if app.input_mode == InputMode::Normal {
                            Style::default().yellow()
                        } else {
                            Style::default()
                        }),
                )
                .wrap(Wrap { trim: false })
                .scroll((app.scroll, 0));

            f.render_widget(chat_paragraph, chunks[0]);

            // 2. Logs Area
            let logs = app.logger.get_logs();
            let log_text: Vec<Line> = logs
                .iter()
                .rev()
                .take(3)
                .map(|l| {
                    Line::from(Span::styled(
                        format!(" > {}", l),
                        Style::default().fg(Color::DarkGray),
                    ))
                })
                .collect();

            let logs_block = Paragraph::new(log_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" System Logs "),
            );
            f.render_widget(logs_block, chunks[1]);

            // 3. Input Area
            let input_title = match app.input_mode {
                InputMode::Normal => " Press 'i' to type",
                InputMode::Editing => " Your Message (Enter to send, Esc to exit typing mode)",
            };

            let input = Paragraph::new(app.input.as_str())
                .style(match app.input_mode {
                    InputMode::Normal => Style::default().fg(Color::DarkGray),
                    InputMode::Editing => Style::default().fg(Color::White),
                })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(input_title)
                        .border_style(if app.input_mode == InputMode::Editing {
                            Style::default().yellow()
                        } else {
                            Style::default()
                        }),
                );
            f.render_widget(input, chunks[2]);

            if app.input_mode == InputMode::Editing {
                f.set_cursor_position((chunks[2].x + app.input.len() as u16 + 1, chunks[2].y + 1));
            }
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('i') => {
                            app.input_mode = InputMode::Editing;
                        }
                        KeyCode::Char('q') => {
                            break;
                        }
                        KeyCode::Up => {
                            app.scroll = app.scroll.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            app.scroll = app.scroll.saturating_add(1);
                        }
                        _ => {}
                    },
                    InputMode::Editing => match key.code {
                        KeyCode::Char('c')
                            if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                        {
                            break;
                        }
                        KeyCode::Enter => {
                            if !app.input.is_empty() {
                                let input_text = app.input.clone();
                                app.messages.push(ChatMessage {
                                    author: MessageAuthor::User,
                                    text: input_text.clone(),
                                });
                                app.is_loading = true;
                                if input_text == "quit" || input_text == "exit" {
                                    break;
                                }
                                let _ = app.tx.send(AppEvent::Input(input_text));
                                app.input.clear();
                            }
                        }
                        KeyCode::Char(c) => {
                            app.input.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input.pop();
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        _ => {}
                    },
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

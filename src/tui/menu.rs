use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Terminal,
};
use std::io::stdout;
use std::time::Duration;
use url::Url;

use super::art::{BREATH_OF_LIFE_ASCII, ITEM_NAME, ITEM_QUOTE};
use crate::cli::{CliArgs, HttpVersion};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    QuickCheck,  // 10 workers, 5s
    Standard,    // 50 workers, 10s
    StressTest,  // 200 workers, 30s
    RateLimited, // 20 workers, 10s, 500 RPS
    Custom,      // Configured in settings
}

impl Preset {
    pub fn name(&self) -> &'static str {
        match self {
            Preset::QuickCheck => "1. Quick Health Check (10 workers, 5s)",
            Preset::Standard => "2. Standard Benchmark (50 workers, 10s)",
            Preset::StressTest => "3. Heavy Stress Test (200 workers, 30s)",
            Preset::RateLimited => "4. Rate Limited Test (20 workers, 500 RPS, 10s)",
            Preset::Custom => "5. Custom Configuration (edit settings below)",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Preset::QuickCheck => {
                "Fast latency sanity check without putting high pressure on target."
            }
            Preset::Standard => {
                "Balanced benchmark measuring normal load throughput and percentiles."
            }
            Preset::StressTest => {
                "High concurrency stress test pushing server connections to limits."
            }
            Preset::RateLimited => {
                "Controlled 500 req/s rate test to monitor latency under steady load."
            }
            Preset::Custom => {
                "Manually fine-tune concurrency, duration, RPS rate, and HTTP method."
            }
        }
    }
}

pub struct MenuState {
    pub target_url: String,
    pub selected_preset: usize,
    pub presets: Vec<Preset>,
    pub concurrency: usize,
    pub duration_secs: u64,
    pub rate_limit: u64,
    pub method: String,
    pub http2: bool,
    pub tui_mode: bool,
    pub active_field: usize, // 0: Presets, 1: URL, 2: Concurrency, 3: Duration, 4: Rate, 5: Method, 6: HTTP2, 7: TUI mode
    pub is_editing_url: bool,
    pub status_msg: Option<String>,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            target_url: "http://127.0.0.1:8080/".to_string(),
            selected_preset: 1, // Standard by default
            presets: vec![
                Preset::QuickCheck,
                Preset::Standard,
                Preset::StressTest,
                Preset::RateLimited,
                Preset::Custom,
            ],
            concurrency: 50,
            duration_secs: 10,
            rate_limit: 0,
            method: "GET".to_string(),
            http2: false,
            tui_mode: true,
            active_field: 0,
            is_editing_url: false,
            status_msg: None,
        }
    }
}

pub fn run_interactive_menu() -> Result<Option<CliArgs>> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = MenuState::default();

    let res = main_menu_loop(&mut terminal, &mut state);

    // Restore terminal cleanly
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

fn main_menu_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    state: &mut MenuState,
) -> Result<Option<CliArgs>> {
    loop {
        terminal.draw(|f| {
            let area = f.area();
            f.render_widget(Clear, area);

            let main_chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(3),  // Header
                    Constraint::Min(18),    // Body (Left Art + Right Menu)
                    Constraint::Length(3),  // Help / Keybindings footer
                ])
                .split(area);

            // Top Header
            let header = Paragraph::new("🌪️ GALE - Modern HTTP Load Generator & Benchmarker")
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .alignment(Alignment::Center)
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(header, main_chunks[0]);

            // Middle layout: Left (Breath of life item) | Right (Menu & config)
            let body_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(38), // ASCII Art column
                    Constraint::Min(45),    // Options & presets column
                ])
                .split(main_chunks[1]);

            // Left: Isaac Breath of Life artwork
            let mut art_lines = Vec::new();
            art_lines.push(Line::from(vec![
                Span::styled(format!("★ {} ★", ITEM_NAME), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            ]));
            art_lines.push(Line::from(vec![
                Span::styled(format!("\"{}\"", ITEM_QUOTE), Style::default().fg(Color::Cyan).add_modifier(Modifier::ITALIC)),
            ]));
            art_lines.push(Line::from(""));

            for line in BREATH_OF_LIFE_ASCII {
                art_lines.push(Line::from(Span::styled(*line, Style::default().fg(Color::LightCyan))));
            }

            art_lines.push(Line::from(""));
            art_lines.push(Line::from(vec![
                Span::styled("Ready to inhale traffic!", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            ]));

            let art_box = Paragraph::new(art_lines)
                .block(Block::default().borders(Borders::ALL).title(" The Binding of Isaac "))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
            f.render_widget(art_box, body_chunks[0]);

            // Right side: Preset selection and configuration
            let right_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(8),  // Presets list
                    Constraint::Min(9),     // Settings editor
                ])
                .split(body_chunks[1]);

            // Presets List
            let preset_items: Vec<ListItem> = state
                .presets
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let is_active_category = state.active_field == 0;
                    let is_selected = i == state.selected_preset;

                    let prefix = if is_selected { "▶ " } else { "  " };
                    let style = if is_selected && is_active_category {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else if is_selected {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Gray)
                    };

                    ListItem::new(Line::from(vec![
                        Span::styled(prefix, style),
                        Span::styled(p.name(), style),
                    ]))
                })
                .collect();

            let active_preset_desc = state.presets[state.selected_preset].description();
            let presets_block = List::new(preset_items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(Span::styled(
                            format!(" Select Test Preset ({}) ", active_preset_desc),
                            Style::default().fg(Color::LightGreen),
                        )),
                );
            f.render_widget(presets_block, right_chunks[0]);

            let mut settings_lines = Vec::new();

            let field_style = |field_idx: usize| {
                if state.active_field == field_idx {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                }
            };

            let pointer = |field_idx: usize| {
                if state.active_field == field_idx { "▶ " } else { "  " }
            };

            // 1. URL
            let url_edit_indicator = if state.is_editing_url { " [Typing... Enter to save]" } else { " (Press [e] to edit)" };
            settings_lines.push(Line::from(vec![
                Span::styled(pointer(1), field_style(1)),
                Span::styled("Target URL:      ", field_style(1)),
                Span::styled(&state.target_url, Style::default().fg(Color::Yellow)),
                Span::styled(url_edit_indicator, Style::default().fg(Color::DarkGray)),
            ]));

            // 2. Concurrency
            settings_lines.push(Line::from(vec![
                Span::styled(pointer(2), field_style(2)),
                Span::styled("Concurrency:     ", field_style(2)),
                Span::styled(format!("{} workers  [Left/Right to adjust]", state.concurrency), Style::default().fg(Color::Green)),
            ]));

            // 3. Duration
            settings_lines.push(Line::from(vec![
                Span::styled(pointer(3), field_style(3)),
                Span::styled("Duration:        ", field_style(3)),
                Span::styled(format!("{}s  [Left/Right to adjust]", state.duration_secs), Style::default().fg(Color::Green)),
            ]));

            // 4. Rate Limit
            let rate_str = if state.rate_limit > 0 {
                format!("{} req/s", state.rate_limit)
            } else {
                "Unlimited".to_string()
            };
            settings_lines.push(Line::from(vec![
                Span::styled(pointer(4), field_style(4)),
                Span::styled("Rate Limit:      ", field_style(4)),
                Span::styled(format!("{}  [Left/Right to adjust]", rate_str), Style::default().fg(Color::Green)),
            ]));

            // 5. Method
            settings_lines.push(Line::from(vec![
                Span::styled(pointer(5), field_style(5)),
                Span::styled("HTTP Method:     ", field_style(5)),
                Span::styled(format!("{}  [Space to toggle GET/POST]", state.method), Style::default().fg(Color::Green)),
            ]));

            // 6. HTTP Version
            settings_lines.push(Line::from(vec![
                Span::styled(pointer(6), field_style(6)),
                Span::styled("HTTP Protocol:   ", field_style(6)),
                Span::styled(
                    format!("{}  [Space to toggle]", if state.http2 { "HTTP/2 Prior Knowledge" } else { "HTTP/1.1" }),
                    Style::default().fg(Color::Green),
                ),
            ]));

            // 7. Visual Mode
            settings_lines.push(Line::from(vec![
                Span::styled(pointer(7), field_style(7)),
                Span::styled("Display Mode:    ", field_style(7)),
                Span::styled(
                    format!("{}  [Space to toggle]", if state.tui_mode { "Interactive TUI (Charts & Isaac Art)" } else { "CLI Progress Bar" }),
                    Style::default().fg(Color::LightCyan),
                ),
            ]));

            if let Some(ref err) = state.status_msg {
                settings_lines.push(Line::from(""));
                settings_lines.push(Line::from(Span::styled(format!("⚠️  {}", err), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))));
            }

            let settings_box = Paragraph::new(settings_lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(Span::styled(" Benchmark Configuration ", Style::default().fg(Color::LightBlue))),
                );
            f.render_widget(settings_box, right_chunks[1]);

            // Footer keybinding guidance
            let footer_text = if state.is_editing_url {
                "Editing URL: Type address, press [Enter] to confirm, [Esc] to cancel"
            } else {
                "Navigation: [↑/↓] Select Field | [←/→] Adjust Value | [Enter] LAUNCH BENCHMARK | [q] Exit"
            };
            let footer = Paragraph::new(footer_text)
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(footer, main_chunks[2]);
        })?;

        // Handle events
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if state.is_editing_url {
                    match key.code {
                        KeyCode::Enter => {
                            state.is_editing_url = false;
                            state.status_msg = None;
                        }
                        KeyCode::Esc => {
                            state.is_editing_url = false;
                        }
                        KeyCode::Backspace => {
                            state.target_url.pop();
                        }
                        KeyCode::Char(c) => {
                            state.target_url.push(c);
                        }
                        _ => {}
                    }
                    continue;
                }

                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        return Ok(None);
                    }
                    KeyCode::Up => {
                        if state.active_field == 0 {
                            if state.selected_preset > 0 {
                                state.selected_preset -= 1;
                                apply_preset(state);
                            }
                        } else {
                            state.active_field = state.active_field.saturating_sub(1);
                        }
                    }
                    KeyCode::Down => {
                        if state.active_field == 0 {
                            if state.selected_preset + 1 < state.presets.len() {
                                state.selected_preset += 1;
                                apply_preset(state);
                            } else {
                                state.active_field = 1;
                            }
                        } else if state.active_field < 7 {
                            state.active_field += 1;
                        }
                    }
                    KeyCode::Left => match state.active_field {
                        2 => state.concurrency = (state.concurrency.saturating_sub(10)).max(1),
                        3 => state.duration_secs = (state.duration_secs.saturating_sub(5)).max(1),
                        4 => state.rate_limit = state.rate_limit.saturating_sub(100),
                        _ => {}
                    },
                    KeyCode::Right => match state.active_field {
                        2 => state.concurrency = (state.concurrency + 10).min(5000),
                        3 => state.duration_secs = (state.duration_secs + 5).min(3600),
                        4 => state.rate_limit += 100,
                        _ => {}
                    },
                    KeyCode::Char(' ') => match state.active_field {
                        5 => {
                            state.method = if state.method == "GET" {
                                "POST".to_string()
                            } else {
                                "GET".to_string()
                            }
                        }
                        6 => state.http2 = !state.http2,
                        7 => state.tui_mode = !state.tui_mode,
                        _ => {}
                    },
                    KeyCode::Char('e') => {
                        if state.active_field == 1 {
                            state.is_editing_url = true;
                        }
                    }
                    KeyCode::Enter => {
                        if state.active_field == 1 {
                            state.is_editing_url = true;
                        } else {
                            // Validate URL and build CliArgs
                            match Url::parse(&state.target_url) {
                                Ok(parsed_url) => {
                                    let args = CliArgs {
                                        url: Some(parsed_url),
                                        concurrency: state.concurrency,
                                        duration: Duration::from_secs(state.duration_secs),
                                        rate_limit: state.rate_limit,
                                        method: state.method.clone(),
                                        headers: Vec::new(),
                                        body: None,
                                        body_file: None,
                                        timeout: Duration::from_secs(5),
                                        http_version: if state.http2 {
                                            HttpVersion::Http2
                                        } else {
                                            HttpVersion::Http1
                                        },
                                        insecure: false,
                                        tui: state.tui_mode,
                                        json_output: None,
                                    };
                                    return Ok(Some(args));
                                }
                                Err(e) => {
                                    state.status_msg = Some(format!("Invalid URL: {}", e));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn apply_preset(state: &mut MenuState) {
    match state.presets[state.selected_preset] {
        Preset::QuickCheck => {
            state.concurrency = 10;
            state.duration_secs = 5;
            state.rate_limit = 0;
        }
        Preset::Standard => {
            state.concurrency = 50;
            state.duration_secs = 10;
            state.rate_limit = 0;
        }
        Preset::StressTest => {
            state.concurrency = 200;
            state.duration_secs = 30;
            state.rate_limit = 0;
        }
        Preset::RateLimited => {
            state.concurrency = 20;
            state.duration_secs = 10;
            state.rate_limit = 500;
        }
        Preset::Custom => {}
    }
}

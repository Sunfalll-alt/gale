pub mod art;

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
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Cell, Chart, Dataset, Gauge, Paragraph, Row, Table, Wrap},
    Terminal,
};
use std::io::stdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Receiver;

use crate::cli::CliArgs;
use crate::stats::{BenchmarkReport, RequestMetric, WorkerStats};
use art::{BREATH_OF_LIFE_ASCII, ITEM_NAME, ITEM_QUOTE};

pub async fn run_tui(
    args: CliArgs,
    mut rx: Receiver<RequestMetric>,
    is_running: Arc<AtomicBool>,
) -> Result<BenchmarkReport> {
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let test_duration = args.duration;
    let url_str = args.url.to_string();
    let concurrency = args.concurrency;

    let start_time = Instant::now();
    let mut stats = WorkerStats::new();

    // Time-series data for RPS chart: (seconds_since_start, rps_in_that_second)
    let mut rps_points: Vec<(f64, f64)> = Vec::new();
    let mut requests_in_current_sec: u64 = 0;
    let mut last_sec_tick = start_time;

    let tick_rate = Duration::from_millis(100);
    let mut last_ui_render = Instant::now();

    loop {
        // Drain newly received metrics from workers
        while let Ok(metric) = rx.try_recv() {
            requests_in_current_sec += 1;
            stats.record(metric);
        }

        let elapsed = start_time.elapsed();
        if elapsed >= test_duration || !is_running.load(Ordering::Relaxed) {
            is_running.store(false, Ordering::SeqCst);
            break;
        }

        // Update RPS history every second
        if last_sec_tick.elapsed() >= Duration::from_secs(1) {
            let sec_f = elapsed.as_secs_f64();
            let current_rps = requests_in_current_sec as f64;
            rps_points.push((sec_f, current_rps));
            if rps_points.len() > 60 {
                rps_points.remove(0);
            }
            requests_in_current_sec = 0;
            last_sec_tick = Instant::now();
        }

        // Handle user keyboard inputs (e.g. 'q' or Esc to cancel early)
        if event::poll(Duration::from_millis(10))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                    is_running.store(false, Ordering::SeqCst);
                    break;
                }
            }
        }

        // Render UI
        if last_ui_render.elapsed() >= tick_rate {
            let progress_ratio =
                (elapsed.as_secs_f64() / test_duration.as_secs_f64()).clamp(0.0, 1.0);
            let total_reqs = stats.total_requests;
            let total_errs = stats.errors;
            let curr_rps = if elapsed.as_secs_f64() > 0.05 {
                total_reqs as f64 / elapsed.as_secs_f64()
            } else {
                0.0
            };

            let p50 = stats
                .histogram
                .as_ref()
                .map(|h| h.value_at_percentile(50.0) as f64 / 1000.0)
                .unwrap_or(0.0);
            let p99 = stats
                .histogram
                .as_ref()
                .map(|h| h.value_at_percentile(99.0) as f64 / 1000.0)
                .unwrap_or(0.0);

            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .margin(1)
                    .constraints([
                        Constraint::Length(3), // Header / Progress
                        Constraint::Min(16),   // Main workspace (ASCII Art + Stats & Chart)
                        Constraint::Length(3), // Footer keys
                    ])
                    .split(f.area());

                // Top Progress Bar
                let gauge_label = format!(
                    "⏱️ {:.1}s / {:.1}s ({:.1}%) | Concurrency: {}",
                    elapsed.as_secs_f64(),
                    test_duration.as_secs_f64(),
                    progress_ratio * 100.0,
                    concurrency
                );
                let gauge = Gauge::default()
                    .block(
                        Block::default().borders(Borders::ALL).title(Span::styled(
                            "🌪️ Gale Benchmark Runner",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        )),
                    )
                    .gauge_style(
                        Style::default()
                            .fg(Color::LightCyan)
                            .bg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD),
                    )
                    .ratio(progress_ratio)
                    .label(gauge_label);
                f.render_widget(gauge, chunks[0]);

                // Middle area split: Left (Art & Item quote) + Right (Stats & Live Chart)
                let main_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(38), // Breath of life artwork & stats
                        Constraint::Min(40),    // Metrics table & Live RPS Chart
                    ])
                    .split(chunks[1]);

                // Left Box: Binding of Isaac Item Card
                let mut art_lines = Vec::new();
                art_lines.push(Line::from(vec![Span::styled(
                    format!("★ {} ★", ITEM_NAME),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )]));
                art_lines.push(Line::from(vec![Span::styled(
                    format!("\"{}\"", ITEM_QUOTE),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::ITALIC),
                )]));
                art_lines.push(Line::from(""));

                for line in BREATH_OF_LIFE_ASCII {
                    art_lines.push(Line::from(Span::styled(
                        *line,
                        Style::default().fg(Color::LightCyan),
                    )));
                }

                art_lines.push(Line::from(""));
                art_lines.push(Line::from(vec![
                    Span::raw("Status: "),
                    Span::styled(
                        if progress_ratio >= 1.0 {
                            "FINISHED"
                        } else {
                            "EXHALING..."
                        },
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));

                let art_paragraph = Paragraph::new(art_lines)
                    .block(Block::default().borders(Borders::ALL).title(Span::styled(
                        " Isaac Item: Breath of Life ",
                        Style::default().fg(Color::LightBlue),
                    )))
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: true });
                f.render_widget(art_paragraph, main_chunks[0]);

                // Right Area split into Stats Table and Live Chart
                let right_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(8), // Stats summary table
                        Constraint::Min(8),    // RPS Chart
                    ])
                    .split(main_chunks[1]);

                // Live Stats Table
                let rows = vec![
                    Row::new(vec![
                        Cell::from("Target URL"),
                        Cell::from(url_str.clone()).style(Style::default().fg(Color::Yellow)),
                    ]),
                    Row::new(vec![
                        Cell::from("Total Requests"),
                        Cell::from(format!("{}", total_reqs))
                            .style(Style::default().fg(Color::White)),
                    ]),
                    Row::new(vec![
                        Cell::from("Current Throughput"),
                        Cell::from(format!("{:.1} req/s", curr_rps)).style(
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Row::new(vec![
                        Cell::from("Latency Median (p50)"),
                        Cell::from(format!("{:.2} ms", p50))
                            .style(Style::default().fg(Color::Cyan)),
                    ]),
                    Row::new(vec![
                        Cell::from("Latency Tail (p99)"),
                        Cell::from(format!("{:.2} ms", p99))
                            .style(Style::default().fg(Color::Magenta)),
                    ]),
                    Row::new(vec![
                        Cell::from("Errors / Failures"),
                        Cell::from(format!("{}", total_errs)).style(if total_errs == 0 {
                            Style::default().fg(Color::Green)
                        } else {
                            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                        }),
                    ]),
                ];

                let table = Table::new(rows, [Constraint::Length(22), Constraint::Min(25)]).block(
                    Block::default().borders(Borders::ALL).title(Span::styled(
                        " Realtime Metrics ",
                        Style::default().fg(Color::Green),
                    )),
                );
                f.render_widget(table, right_chunks[0]);

                // Live Throughput Chart
                let max_rps_val = rps_points.iter().map(|(_, r)| *r).fold(10.0, f64::max) * 1.25;
                let max_sec_val = elapsed.as_secs_f64().max(10.0);

                let datasets = vec![Dataset::default()
                    .name("Requests/Sec")
                    .marker(symbols::Marker::Braille)
                    .style(Style::default().fg(Color::LightGreen))
                    .data(&rps_points)];

                let chart = Chart::new(datasets)
                    .block(Block::default().borders(Borders::ALL).title(Span::styled(
                        " Throughput Trend (RPS) ",
                        Style::default().fg(Color::LightGreen),
                    )))
                    .x_axis(
                        Axis::default()
                            .title("Time (s)")
                            .style(Style::default().fg(Color::Gray))
                            .bounds([0.0, max_sec_val])
                            .labels(vec![
                                Span::raw("0s"),
                                Span::raw(format!("{:.0}s", max_sec_val / 2.0)),
                                Span::raw(format!("{:.0}s", max_sec_val)),
                            ]),
                    )
                    .y_axis(
                        Axis::default()
                            .title("RPS")
                            .style(Style::default().fg(Color::Gray))
                            .bounds([0.0, max_rps_val])
                            .labels(vec![
                                Span::raw("0"),
                                Span::raw(format!("{:.0}", max_rps_val / 2.0)),
                                Span::raw(format!("{:.0}", max_rps_val)),
                            ]),
                    );
                f.render_widget(chart, right_chunks[1]);

                // Footer keybindings
                let footer =
                    Paragraph::new("Press [q] or [Esc] to cancel benchmark and compile report")
                        .style(Style::default().fg(Color::DarkGray))
                        .alignment(Alignment::Center)
                        .block(Block::default().borders(Borders::ALL));
                f.render_widget(footer, chunks[2]);
            })?;
            last_ui_render = Instant::now();
        }
    }

    // Drain remaining metrics in channel
    while let Ok(metric) = rx.try_recv() {
        stats.record(metric);
    }

    // Restore terminal before exiting
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    let final_report = BenchmarkReport::from_stats(
        stats,
        start_time.elapsed(),
        args.url.to_string(),
        args.concurrency,
    );

    Ok(final_report)
}

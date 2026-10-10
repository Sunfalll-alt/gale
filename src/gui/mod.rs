use eframe::egui;
use egui_plot::{Line, Plot, PlotPoints};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Receiver;
use url::Url;

use crate::cli::{CliArgs, HttpVersion};
use crate::stats::{BenchmarkReport, RequestMetric, WorkerStats};

pub fn run_gui_app(initial_args: CliArgs) -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1020.0, 720.0])
            .with_min_inner_size([760.0, 540.0])
            .with_title("🌪️ Gale - Modern HTTP Benchmark"),
        ..Default::default()
    };

    eframe::run_native(
        "Gale - HTTP Benchmarking Tool",
        native_options,
        Box::new(|cc| {
            setup_custom_styles(&cc.egui_ctx);
            Ok(Box::new(GaleGuiApp::new(initial_args)))
        }),
    )
}

fn setup_custom_styles(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_corner_radius = 10.into();
    visuals.widgets.noninteractive.corner_radius = 8.into();
    visuals.widgets.inactive.corner_radius = 8.into();
    visuals.widgets.hovered.corner_radius = 8.into();
    visuals.widgets.active.corner_radius = 8.into();

    // Friendly, balanced dark palette
    visuals.panel_fill = egui::Color32::from_rgb(20, 24, 30);
    visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(28, 34, 44);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(34, 42, 54);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(46, 56, 72);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(58, 72, 94);

    ctx.set_visuals(visuals);
}

enum AppState {
    Configuring,
    Running {
        start_time: Instant,
        duration: Duration,
        is_running_flag: Arc<AtomicBool>,
        metric_rx: Receiver<RequestMetric>,
        stats: WorkerStats,
        rps_history: Vec<[f64; 2]>,
        last_sec_tick: Instant,
        current_sec_requests: u64,
    },
    Finished {
        report: BenchmarkReport,
    },
}

pub struct GaleGuiApp {
    url_input: String,
    concurrency: usize,
    duration_secs: u64,
    rate_limit: u64,
    method: String,
    http2: bool,
    error_msg: Option<String>,
    app_state: AppState,
}

impl GaleGuiApp {
    pub fn new(args: CliArgs) -> Self {
        let url_str = args
            .url
            .map(|u| u.to_string())
            .unwrap_or_else(|| "http://127.0.0.1:8080/".to_string());

        Self {
            url_input: url_str,
            concurrency: args.concurrency,
            duration_secs: args.duration.as_secs(),
            rate_limit: args.rate_limit,
            method: args.method,
            http2: args.http_version == HttpVersion::Http2,
            error_msg: None,
            app_state: AppState::Configuring,
        }
    }

    fn start_benchmark(&mut self) {
        let parsed_url = match Url::parse(&self.url_input) {
            Ok(u) => u,
            Err(e) => {
                self.error_msg = Some(format!("Invalid URL: {}", e));
                return;
            }
        };

        self.error_msg = None;
        let duration = Duration::from_secs(self.duration_secs);
        let concurrency = self.concurrency;
        let rate_limit = self.rate_limit;
        let method = self.method.clone();
        let _http2 = self.http2;

        let (tx, rx) = tokio::sync::mpsc::channel::<RequestMetric>(concurrency * 100);
        let is_running_flag = Arc::new(AtomicBool::new(true));
        let flag_clone = Arc::clone(&is_running_flag);

        // Spawn workers in background tokio runtime
        let client_url = parsed_url.clone();
        tokio::spawn(async move {
            let client = match reqwest::Client::builder()
                .tcp_nodelay(true)
                .pool_max_idle_per_host(concurrency * 2)
                .timeout(Duration::from_secs(5))
                .build()
            {
                Ok(c) => c,
                Err(_) => return,
            };

            let rate_interval = (1_000_000 * concurrency as u64)
                .checked_div(rate_limit)
                .map(|micros| Duration::from_micros(micros.max(1)));

            let mut handles = Vec::with_capacity(concurrency);
            for _ in 0..concurrency {
                let client = client.clone();
                let client_url = client_url.clone();
                let method = method.clone();
                let tx = tx.clone();
                let flag = Arc::clone(&flag_clone);

                let handle = tokio::spawn(async move {
                    while flag.load(Ordering::Relaxed) {
                        let req_start = Instant::now();
                        let http_method = match method.as_str() {
                            "POST" => reqwest::Method::POST,
                            "PUT" => reqwest::Method::PUT,
                            "DELETE" => reqwest::Method::DELETE,
                            _ => reqwest::Method::GET,
                        };

                        let req = client.request(http_method, client_url.clone());
                        match req.send().await {
                            Ok(res) => {
                                let status = res.status().as_u16();
                                let is_error = res.error_for_status_ref().is_err();
                                let body_bytes = res.bytes().await.map(|b| b.len()).unwrap_or(0);
                                let latency = req_start.elapsed();

                                if tx
                                    .send(RequestMetric {
                                        latency,
                                        status: Some(status),
                                        bytes: body_bytes,
                                        is_error,
                                    })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(_) => {
                                let latency = req_start.elapsed();
                                if tx
                                    .send(RequestMetric {
                                        latency,
                                        status: None,
                                        bytes: 0,
                                        is_error: true,
                                    })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }

                        if let Some(interval) = rate_interval {
                            tokio::time::sleep(interval).await;
                        }
                    }
                });
                handles.push(handle);
            }
        });

        self.app_state = AppState::Running {
            start_time: Instant::now(),
            duration,
            is_running_flag,
            metric_rx: rx,
            stats: WorkerStats::new(),
            rps_history: Vec::new(),
            last_sec_tick: Instant::now(),
            current_sec_requests: 0,
        };
    }
}

impl eframe::App for GaleGuiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Continuous repaint while benchmarking
        if matches!(self.app_state, AppState::Running { .. }) {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }

        // Top App Bar
        egui::Frame::new()
            .fill(egui::Color32::from_rgb(26, 32, 42))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("🌪️ Gale")
                            .size(20.0)
                            .strong()
                            .color(egui::Color32::from_rgb(100, 200, 255)),
                    );
                    ui.label(
                        egui::RichText::new("HTTP Load Generator & Benchmarking")
                            .size(13.0)
                            .color(egui::Color32::from_rgb(150, 165, 185)),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        egui::widgets::global_theme_preference_switch(ui);
                    });
                });
            });

        ui.add_space(10.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(6.0);
            match &mut self.app_state {
                AppState::Configuring => {
                    // Hero Card: Target & Presets
                    egui::Frame::new()
                        .corner_radius(12)
                        .fill(egui::Color32::from_rgb(26, 32, 44))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(42, 52, 70)))
                        .inner_margin(18)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("🎯 Target Destination")
                                    .size(16.0)
                                    .strong()
                                    .color(egui::Color32::from_rgb(220, 230, 245)),
                            );
                            ui.add_space(4.0);

                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("URL:").strong());
                                let text_edit = egui::TextEdit::singleline(&mut self.url_input)
                                    .desired_width(ui.available_width() - 8.0)
                                    .hint_text("http://127.0.0.1:8080/api");
                                ui.add(text_edit);
                            });

                            ui.add_space(14.0);
                            ui.label(
                                egui::RichText::new("⚡ Quick Test Presets")
                                    .size(14.0)
                                    .strong()
                                    .color(egui::Color32::from_rgb(180, 200, 225)),
                            );
                            ui.add_space(4.0);

                            ui.horizontal_wrapped(|ui| {
                                preset_chip(ui, "🟢 Quick Check", "10w, 5s", || {
                                    self.concurrency = 10;
                                    self.duration_secs = 5;
                                    self.rate_limit = 0;
                                });
                                preset_chip(ui, "🔵 Standard", "50w, 10s", || {
                                    self.concurrency = 50;
                                    self.duration_secs = 10;
                                    self.rate_limit = 0;
                                });
                                preset_chip(ui, "🔴 Heavy Stress", "200w, 30s", || {
                                    self.concurrency = 200;
                                    self.duration_secs = 30;
                                    self.rate_limit = 0;
                                });
                                preset_chip(ui, "🟣 Rate Limited", "20w, 500 RPS", || {
                                    self.concurrency = 20;
                                    self.duration_secs = 10;
                                    self.rate_limit = 500;
                                });
                            });
                        });

                    ui.add_space(14.0);

                    // Fine-tuning Settings Grid Card
                    egui::Frame::new()
                        .corner_radius(12)
                        .fill(egui::Color32::from_rgb(26, 32, 44))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(42, 52, 70)))
                        .inner_margin(18)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("⚙️ Advanced Parameters")
                                    .size(16.0)
                                    .strong()
                                    .color(egui::Color32::from_rgb(220, 230, 245)),
                            );
                            ui.add_space(8.0);

                            egui::Grid::new("config_grid")
                                .num_columns(2)
                                .spacing([20.0, 10.0])
                                .show(ui, |ui| {
                                    ui.label("Concurrency (Workers):");
                                    ui.add(
                                        egui::DragValue::new(&mut self.concurrency).range(1..=5000),
                                    );
                                    ui.end_row();

                                    ui.label("Duration (Seconds):");
                                    ui.add(
                                        egui::DragValue::new(&mut self.duration_secs)
                                            .range(1..=3600),
                                    );
                                    ui.end_row();

                                    ui.label("Target Rate Limit (RPS):");
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            egui::DragValue::new(&mut self.rate_limit)
                                                .range(0..=100000),
                                        );
                                        ui.label(
                                            egui::RichText::new(if self.rate_limit == 0 {
                                                "(Unlimited)"
                                            } else {
                                                "capped"
                                            })
                                            .color(egui::Color32::GRAY)
                                            .italics(),
                                        );
                                    });
                                    ui.end_row();

                                    ui.label("HTTP Method:");
                                    egui::ComboBox::from_label("")
                                        .selected_text(&self.method)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut self.method,
                                                "GET".to_string(),
                                                "GET",
                                            );
                                            ui.selectable_value(
                                                &mut self.method,
                                                "POST".to_string(),
                                                "POST",
                                            );
                                            ui.selectable_value(
                                                &mut self.method,
                                                "PUT".to_string(),
                                                "PUT",
                                            );
                                            ui.selectable_value(
                                                &mut self.method,
                                                "DELETE".to_string(),
                                                "DELETE",
                                            );
                                        });
                                    ui.end_row();

                                    ui.label("HTTP Protocol:");
                                    ui.checkbox(&mut self.http2, "HTTP/2 Prior Knowledge");
                                    ui.end_row();
                                });
                        });

                    if let Some(ref err) = self.error_msg {
                        ui.add_space(8.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 100, 100),
                            format!("⚠️ {}", err),
                        );
                    }

                    ui.add_space(16.0);
                    let start_btn = egui::Button::new(
                        egui::RichText::new("🚀 LAUNCH BENCHMARK")
                            .size(15.0)
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(egui::Color32::from_rgb(40, 130, 220));

                    if ui
                        .add_sized([ui.available_width(), 44.0], start_btn)
                        .clicked()
                    {
                        self.start_benchmark();
                    }
                }

                AppState::Running {
                    start_time,
                    duration,
                    is_running_flag,
                    metric_rx,
                    stats,
                    rps_history,
                    last_sec_tick,
                    current_sec_requests,
                } => {
                    while let Ok(m) = metric_rx.try_recv() {
                        *current_sec_requests += 1;
                        stats.record(m);
                    }

                    let elapsed = start_time.elapsed();
                    let progress = (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0);

                    if last_sec_tick.elapsed() >= Duration::from_secs(1) {
                        let sec_val = elapsed.as_secs_f64();
                        let rps_val = *current_sec_requests as f64;
                        rps_history.push([sec_val, rps_val]);
                        *current_sec_requests = 0;
                        *last_sec_tick = Instant::now();
                    }

                    let mut should_finish = elapsed >= *duration;

                    // Progress Header
                    egui::Frame::new()
                        .corner_radius(10)
                        .fill(egui::Color32::from_rgb(26, 32, 44))
                        .inner_margin(14)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("⚡ Running Test on:")
                                        .size(15.0)
                                        .strong(),
                                );
                                ui.label(
                                    egui::RichText::new(&self.url_input)
                                        .color(egui::Color32::from_rgb(240, 200, 100))
                                        .strong(),
                                );

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let cancel_btn = egui::Button::new("🛑 Stop & View Report")
                                            .fill(egui::Color32::from_rgb(180, 50, 60));
                                        if ui.add(cancel_btn).clicked() {
                                            should_finish = true;
                                        }
                                    },
                                );
                            });

                            ui.add_space(8.0);
                            let pb = egui::ProgressBar::new(progress as f32)
                                .show_percentage()
                                .animate(true)
                                .fill(egui::Color32::from_rgb(40, 150, 240));
                            ui.add(pb);
                        });

                    if should_finish {
                        is_running_flag.store(false, Ordering::SeqCst);
                        let final_stats = std::mem::take(stats);
                        let report = BenchmarkReport::from_stats(
                            final_stats,
                            elapsed,
                            self.url_input.clone(),
                            self.concurrency,
                        );
                        self.app_state = AppState::Finished { report };
                        return;
                    }

                    ui.add_space(12.0);

                    // Real-time metric cards
                    let curr_rps = if elapsed.as_secs_f64() > 0.1 {
                        stats.total_requests as f64 / elapsed.as_secs_f64()
                    } else {
                        0.0
                    };
                    let p50 = stats
                        .histogram
                        .as_ref()
                        .map(|h| h.value_at_percentile(50.0) as f64 / 1000.0)
                        .unwrap_or(0.0);

                    ui.columns(4, |cols| {
                        styled_metric_card(
                            &mut cols[0],
                            "Total Requests",
                            format!("{}", stats.total_requests),
                            egui::Color32::from_rgb(220, 230, 250),
                        );
                        styled_metric_card(
                            &mut cols[1],
                            "Throughput",
                            format!("{:.1} req/s", curr_rps),
                            egui::Color32::from_rgb(80, 220, 130),
                        );
                        styled_metric_card(
                            &mut cols[2],
                            "Latency (p50)",
                            format!("{:.2} ms", p50),
                            egui::Color32::from_rgb(100, 200, 255),
                        );
                        styled_metric_card(
                            &mut cols[3],
                            "Errors",
                            format!("{}", stats.errors),
                            if stats.errors == 0 {
                                egui::Color32::from_rgb(100, 220, 140)
                            } else {
                                egui::Color32::from_rgb(250, 80, 90)
                            },
                        );
                    });

                    ui.add_space(14.0);

                    // Real-time Throughput Chart with styling
                    egui::Frame::new()
                        .corner_radius(12)
                        .fill(egui::Color32::from_rgb(24, 30, 40))
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("📈 Real-Time Requests Per Second (RPS)")
                                    .strong()
                                    .color(egui::Color32::from_rgb(180, 200, 220)),
                            );
                            ui.add_space(6.0);

                            let points = PlotPoints::from(rps_history.clone());
                            let line = Line::new("RPS", points)
                                .color(egui::Color32::from_rgb(70, 210, 140))
                                .width(2.5);

                            Plot::new("live_rps_plot")
                                .height(280.0)
                                .x_axis_label("Elapsed Time (s)")
                                .y_axis_label("RPS")
                                .show(ui, |plot_ui| {
                                    plot_ui.line(line);
                                });
                        });
                }

                AppState::Finished { report } => {
                    let mut rerun_test = false;

                    // Celebration Banner
                    egui::Frame::new()
                        .corner_radius(12)
                        .fill(egui::Color32::from_rgb(26, 40, 34))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(40, 100, 60)))
                        .inner_margin(16)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("🎉 Benchmark Completed Successfully!")
                                        .size(18.0)
                                        .strong()
                                        .color(egui::Color32::from_rgb(100, 240, 150)),
                                );

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let rerun_btn = egui::Button::new("🔄 Run Another Test")
                                            .fill(egui::Color32::from_rgb(40, 130, 220));
                                        if ui.add(rerun_btn).clicked() {
                                            rerun_test = true;
                                        }
                                    },
                                );
                            });
                        });

                    if rerun_test {
                        self.app_state = AppState::Configuring;
                        return;
                    }

                    ui.add_space(14.0);

                    // Results Cards
                    ui.columns(4, |cols| {
                        styled_metric_card(
                            &mut cols[0],
                            "Total Requests",
                            format!("{}", report.total_requests),
                            egui::Color32::from_rgb(220, 230, 250),
                        );
                        styled_metric_card(
                            &mut cols[1],
                            "Throughput",
                            format!("{:.1} req/s", report.requests_per_sec),
                            egui::Color32::from_rgb(80, 220, 130),
                        );
                        styled_metric_card(
                            &mut cols[2],
                            "Transfer Speed",
                            format!("{:.2} MB/s", report.bytes_per_sec / 1024.0 / 1024.0),
                            egui::Color32::from_rgb(100, 200, 255),
                        );
                        styled_metric_card(
                            &mut cols[3],
                            "Failed Requests",
                            format!("{}", report.error_requests),
                            if report.error_requests == 0 {
                                egui::Color32::from_rgb(100, 220, 140)
                            } else {
                                egui::Color32::from_rgb(250, 80, 90)
                            },
                        );
                    });

                    ui.add_space(14.0);

                    // Detailed Latency Breakdown Card
                    egui::Frame::new()
                        .corner_radius(12)
                        .fill(egui::Color32::from_rgb(26, 32, 44))
                        .inner_margin(16)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("⏱️ HDR Latency Distribution")
                                    .size(16.0)
                                    .strong()
                                    .color(egui::Color32::from_rgb(220, 230, 245)),
                            );
                            ui.add_space(8.0);

                            egui::Grid::new("latency_results_grid")
                                .striped(true)
                                .min_col_width(120.0)
                                .spacing([30.0, 8.0])
                                .show(ui, |ui| {
                                    ui.label("Min Latency:");
                                    ui.label(format!("{:.3} ms", report.latency.min_ms));
                                    ui.label("50% (Median):");
                                    ui.label(format!("{:.3} ms", report.latency.p50_ms));
                                    ui.end_row();

                                    ui.label("75% Percentile:");
                                    ui.label(format!("{:.3} ms", report.latency.p75_ms));
                                    ui.label("90% Percentile:");
                                    ui.label(format!("{:.3} ms", report.latency.p90_ms));
                                    ui.end_row();

                                    ui.label("99% Percentile:");
                                    ui.colored_label(
                                        egui::Color32::from_rgb(240, 200, 100),
                                        format!("{:.3} ms", report.latency.p99_ms),
                                    );
                                    ui.label("99.9% Tail Latency:");
                                    ui.colored_label(
                                        egui::Color32::from_rgb(240, 160, 80),
                                        format!("{:.3} ms", report.latency.p99_9_ms),
                                    );
                                    ui.end_row();

                                    ui.label("Max Latency:");
                                    ui.colored_label(
                                        egui::Color32::from_rgb(250, 90, 90),
                                        format!("{:.3} ms", report.latency.max_ms),
                                    );
                                    ui.label("Mean ± StdDev:");
                                    ui.label(format!(
                                        "{:.3} ms ± {:.3} ms",
                                        report.latency.mean_ms, report.latency.stddev_ms
                                    ));
                                    ui.end_row();
                                });
                        });
                }
            }
        });
    }
}

fn preset_chip(ui: &mut egui::Ui, title: &str, subtitle: &str, mut on_click: impl FnMut()) {
    let btn = egui::Button::new(
        egui::RichText::new(format!("{} ({})", title, subtitle))
            .size(12.5)
            .color(egui::Color32::from_rgb(210, 225, 245)),
    )
    .fill(egui::Color32::from_rgb(36, 44, 58))
    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(52, 64, 84)));

    if ui.add(btn).clicked() {
        on_click();
    }
}

fn styled_metric_card(ui: &mut egui::Ui, title: &str, val: String, val_color: egui::Color32) {
    egui::Frame::new()
        .corner_radius(10)
        .fill(egui::Color32::from_rgb(26, 32, 44))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(42, 52, 70)))
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(title)
                    .size(12.0)
                    .color(egui::Color32::from_rgb(140, 155, 175)),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(val)
                    .size(18.0)
                    .strong()
                    .color(val_color),
            );
        });
}

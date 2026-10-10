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
            .with_inner_size([960.0, 680.0])
            .with_min_inner_size([720.0, 520.0])
            .with_title("Gale - HTTP Benchmarking Tool"),
        ..Default::default()
    };

    eframe::run_native(
        "Gale - HTTP Load Tester",
        native_options,
        Box::new(|_cc| Ok(Box::new(GaleGuiApp::new(initial_args)))),
    )
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
        // Request continuous repaint while benchmarking
        if matches!(self.app_state, AppState::Running { .. }) {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.heading("🌪️ Gale - High-Performance HTTP Load Tester");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::widgets::global_theme_preference_switch(ui);
            });
        });
        ui.separator();

        match &mut self.app_state {
            AppState::Configuring => {
                ui.add_space(8.0);
                ui.group(|ui| {
                    ui.heading("🎯 Target Configuration");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Target URL: ");
                        ui.text_edit_singleline(&mut self.url_input);
                    });

                    ui.add_space(6.0);
                    ui.heading("⚡ Test Presets");
                    ui.horizontal(|ui| {
                        if ui.button("Quick Check (10w, 5s)").clicked() {
                            self.concurrency = 10;
                            self.duration_secs = 5;
                            self.rate_limit = 0;
                        }
                        if ui.button("Standard (50w, 10s)").clicked() {
                            self.concurrency = 50;
                            self.duration_secs = 10;
                            self.rate_limit = 0;
                        }
                        if ui.button("Heavy Stress (200w, 30s)").clicked() {
                            self.concurrency = 200;
                            self.duration_secs = 30;
                            self.rate_limit = 0;
                        }
                        if ui.button("Rate Limited (20w, 500 RPS)").clicked() {
                            self.concurrency = 20;
                            self.duration_secs = 10;
                            self.rate_limit = 500;
                        }
                    });

                    ui.add_space(8.0);
                    ui.heading("⚙️ Settings");
                    ui.horizontal(|ui| {
                        ui.label("Concurrency (workers):");
                        ui.add(egui::DragValue::new(&mut self.concurrency).range(1..=5000));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Duration (seconds):");
                        ui.add(egui::DragValue::new(&mut self.duration_secs).range(1..=3600));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Rate Limit (RPS, 0 for max):");
                        ui.add(egui::DragValue::new(&mut self.rate_limit).range(0..=100000));
                    });
                    ui.horizontal(|ui| {
                        ui.label("HTTP Method:");
                        egui::ComboBox::from_label("")
                            .selected_text(&self.method)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.method, "GET".to_string(), "GET");
                                ui.selectable_value(&mut self.method, "POST".to_string(), "POST");
                                ui.selectable_value(&mut self.method, "PUT".to_string(), "PUT");
                                ui.selectable_value(
                                    &mut self.method,
                                    "DELETE".to_string(),
                                    "DELETE",
                                );
                            });
                    });
                    ui.checkbox(&mut self.http2, "Force HTTP/2 Prior Knowledge");
                });

                if let Some(ref err) = self.error_msg {
                    ui.colored_label(egui::Color32::RED, err);
                }

                ui.add_space(14.0);
                if ui
                    .add_sized(
                        [ui.available_width(), 44.0],
                        egui::Button::new("🚀 START BENCHMARK"),
                    )
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
                // Drain metrics
                while let Ok(m) = metric_rx.try_recv() {
                    *current_sec_requests += 1;
                    stats.record(m);
                }

                let elapsed = start_time.elapsed();
                let progress = (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0);

                // Update RPS chart points each second
                if last_sec_tick.elapsed() >= Duration::from_secs(1) {
                    let sec_val = elapsed.as_secs_f64();
                    let rps_val = *current_sec_requests as f64;
                    rps_history.push([sec_val, rps_val]);
                    *current_sec_requests = 0;
                    *last_sec_tick = Instant::now();
                }

                let mut should_finish = elapsed >= *duration;

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.heading("⚡ Benchmarking in progress...");
                    if ui.button("🛑 Cancel & Compile").clicked() {
                        should_finish = true;
                    }
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

                ui.add(egui::ProgressBar::new(progress as f32).show_percentage());

                ui.add_space(8.0);
                let curr_rps = if elapsed.as_secs_f64() > 0.1 {
                    stats.total_requests as f64 / elapsed.as_secs_f64()
                } else {
                    0.0
                };

                ui.columns(4, |cols| {
                    cols[0].metric("Total Requests", format!("{}", stats.total_requests));
                    cols[1].metric("Current RPS", format!("{:.1} req/s", curr_rps));
                    cols[2].metric("Errors", format!("{}", stats.errors));
                    let p50 = stats
                        .histogram
                        .as_ref()
                        .map(|h| h.value_at_percentile(50.0) as f64 / 1000.0)
                        .unwrap_or(0.0);
                    cols[3].metric("Latency p50", format!("{:.2} ms", p50));
                });

                ui.add_space(10.0);
                ui.label("📈 Realtime Throughput (RPS):");
                let points = PlotPoints::from(rps_history.clone());
                let line = Line::new("RPS", points)
                    .color(egui::Color32::from_rgb(80, 220, 120))
                    .width(2.5);

                Plot::new("live_rps_plot")
                    .height(280.0)
                    .x_axis_label("Time (s)")
                    .y_axis_label("Req/s")
                    .show(ui, |plot_ui| {
                        plot_ui.line(line);
                    });
            }

            AppState::Finished { report } => {
                ui.add_space(8.0);
                ui.heading("✅ Benchmark Completed!");
                ui.add_space(6.0);

                ui.columns(4, |cols| {
                    cols[0].metric("Total Requests", format!("{}", report.total_requests));
                    cols[1].metric(
                        "Throughput",
                        format!("{:.1} req/s", report.requests_per_sec),
                    );
                    cols[2].metric(
                        "Transfer",
                        format!("{:.2} MB/s", report.bytes_per_sec / 1024.0 / 1024.0),
                    );
                    cols[3].metric("Error Count", format!("{}", report.error_requests));
                });

                ui.add_space(10.0);
                ui.group(|ui| {
                    ui.heading("⏱️ Latency Percentiles");
                    egui::Grid::new("latency_grid")
                        .striped(true)
                        .show(ui, |ui| {
                            ui.label("Min:");
                            ui.label(format!("{:.3} ms", report.latency.min_ms));
                            ui.end_row();

                            ui.label("Median (p50):");
                            ui.label(format!("{:.3} ms", report.latency.p50_ms));
                            ui.end_row();

                            ui.label("p90:");
                            ui.label(format!("{:.3} ms", report.latency.p90_ms));
                            ui.end_row();

                            ui.label("p99:");
                            ui.label(format!("{:.3} ms", report.latency.p99_ms));
                            ui.end_row();

                            ui.label("p99.9:");
                            ui.label(format!("{:.3} ms", report.latency.p99_9_ms));
                            ui.end_row();

                            ui.label("Max:");
                            ui.label(format!("{:.3} ms", report.latency.max_ms));
                            ui.end_row();
                        });
                });

                ui.add_space(12.0);
                if ui.button("🔄 Run Another Test").clicked() {
                    self.app_state = AppState::Configuring;
                }
            }
        }
    }
}

trait MetricUiExt {
    fn metric(&mut self, label: &str, value: String);
}

impl MetricUiExt for egui::Ui {
    fn metric(&mut self, label: &str, value: String) {
        self.group(|ui| {
            ui.label(label);
            ui.heading(value);
        });
    }
}

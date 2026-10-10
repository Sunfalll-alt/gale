use colored::Colorize;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table};
use hdrhistogram::Histogram;
use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RequestMetric {
    pub latency: Duration,
    pub status: Option<u16>,
    pub bytes: usize,
    pub is_error: bool,
}

#[derive(Debug, Default)]
pub struct WorkerStats {
    pub histogram: Option<Histogram<u64>>,
    pub total_requests: u64,
    pub total_bytes: u64,
    pub status_codes: HashMap<u16, u64>,
    pub errors: u64,
}

impl WorkerStats {
    pub fn new() -> Self {
        // Track latencies from 1 microsecond to 1 minute with 3 significant figures
        let histogram = Histogram::<u64>::new_with_bounds(1, 60_000_000, 3)
            .unwrap_or_else(|_| Histogram::new(3).unwrap());
        Self {
            histogram: Some(histogram),
            total_requests: 0,
            total_bytes: 0,
            status_codes: HashMap::new(),
            errors: 0,
        }
    }

    pub fn record(&mut self, metric: RequestMetric) {
        self.total_requests += 1;
        self.total_bytes += metric.bytes as u64;

        if metric.is_error {
            self.errors += 1;
        }

        if let Some(code) = metric.status {
            *self.status_codes.entry(code).or_insert(0) += 1;
        }

        let micros = metric.latency.as_micros() as u64;
        let micros_clamped = micros.clamp(1, 60_000_000);
        if let Some(ref mut hist) = self.histogram {
            let _ = hist.record(micros_clamped);
        }
    }

    #[allow(dead_code)]
    pub fn merge(&mut self, other: WorkerStats) {
        self.total_requests += other.total_requests;
        self.total_bytes += other.total_bytes;
        self.errors += other.errors;

        for (code, count) in other.status_codes {
            *self.status_codes.entry(code).or_insert(0) += count;
        }

        if let (Some(ref mut h1), Some(h2)) = (&mut self.histogram, other.histogram) {
            let _ = h1.add(h2);
        }
    }
}

#[derive(Debug, Serialize)]
pub struct LatencySummary {
    pub min_ms: f64,
    pub mean_ms: f64,
    pub stddev_ms: f64,
    pub max_ms: f64,
    pub p50_ms: f64,
    pub p75_ms: f64,
    pub p90_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub p99_9_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct BenchmarkReport {
    pub url: String,
    pub duration_seconds: f64,
    pub concurrency: usize,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub error_requests: u64,
    pub requests_per_sec: f64,
    pub total_bytes: u64,
    pub bytes_per_sec: f64,
    pub status_distribution: HashMap<u16, u64>,
    pub latency: LatencySummary,
}

impl BenchmarkReport {
    pub fn from_stats(
        stats: WorkerStats,
        duration: Duration,
        url: String,
        concurrency: usize,
    ) -> Self {
        let duration_secs = duration.as_secs_f64().max(0.001);
        let req_per_sec = stats.total_requests as f64 / duration_secs;
        let bytes_per_sec = stats.total_bytes as f64 / duration_secs;

        let hist = stats
            .histogram
            .unwrap_or_else(|| Histogram::new(3).unwrap());

        let min_ms = hist.min() as f64 / 1000.0;
        let mean_ms = hist.mean() / 1000.0;
        let stddev_ms = hist.stdev() / 1000.0;
        let max_ms = hist.max() as f64 / 1000.0;

        let p50_ms = hist.value_at_percentile(50.0) as f64 / 1000.0;
        let p75_ms = hist.value_at_percentile(75.0) as f64 / 1000.0;
        let p90_ms = hist.value_at_percentile(90.0) as f64 / 1000.0;
        let p95_ms = hist.value_at_percentile(95.0) as f64 / 1000.0;
        let p99_ms = hist.value_at_percentile(99.0) as f64 / 1000.0;
        let p99_9_ms = hist.value_at_percentile(99.9) as f64 / 1000.0;

        let successful = stats.total_requests.saturating_sub(stats.errors);

        Self {
            url,
            duration_seconds: duration_secs,
            concurrency,
            total_requests: stats.total_requests,
            successful_requests: successful,
            error_requests: stats.errors,
            requests_per_sec: req_per_sec,
            total_bytes: stats.total_bytes,
            bytes_per_sec,
            status_distribution: stats.status_codes,
            latency: LatencySummary {
                min_ms,
                mean_ms,
                stddev_ms,
                max_ms,
                p50_ms,
                p75_ms,
                p90_ms,
                p95_ms,
                p99_ms,
                p99_9_ms,
            },
        }
    }

    pub fn print_terminal(&self) {
        println!();
        println!("{}", "⚡ Benchmark Summary:".bold().cyan());

        let mut overview_table = Table::new();
        overview_table
            .load_preset(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic);

        overview_table.add_row(vec![
            Cell::new("Target URL").fg(Color::Cyan),
            Cell::new(&self.url),
        ]);
        overview_table.add_row(vec![
            Cell::new("Concurrency").fg(Color::Cyan),
            Cell::new(self.concurrency.to_string()),
        ]);
        overview_table.add_row(vec![
            Cell::new("Duration").fg(Color::Cyan),
            Cell::new(format!("{:.2} s", self.duration_seconds)),
        ]);
        overview_table.add_row(vec![
            Cell::new("Total Requests").fg(Color::Cyan),
            Cell::new(self.total_requests.to_string()),
        ]);
        overview_table.add_row(vec![
            Cell::new("Throughput (RPS)")
                .fg(Color::Green)
                .add_attribute(Attribute::Bold),
            Cell::new(format!("{:.2} req/s", self.requests_per_sec))
                .fg(Color::Green)
                .add_attribute(Attribute::Bold),
        ]);
        overview_table.add_row(vec![
            Cell::new("Transfer Rate").fg(Color::Cyan),
            Cell::new(format!("{}/s", format_bytes(self.bytes_per_sec as u64))),
        ]);
        overview_table.add_row(vec![
            Cell::new("Total Data Transferred").fg(Color::Cyan),
            Cell::new(format_bytes(self.total_bytes)),
        ]);

        let success_rate = if self.total_requests > 0 {
            (self.successful_requests as f64 / self.total_requests as f64) * 100.0
        } else {
            0.0
        };

        let success_color = if self.error_requests == 0 {
            Color::Green
        } else {
            Color::Red
        };

        overview_table.add_row(vec![
            Cell::new("Success Rate").fg(Color::Cyan),
            Cell::new(format!(
                "{:.2}% ({} errors)",
                success_rate, self.error_requests
            ))
            .fg(success_color),
        ]);

        println!("{overview_table}");
        println!();

        println!("{}", "⏱️ Latency Distribution:".bold().cyan());
        let mut lat_table = Table::new();
        lat_table.load_preset(UTF8_FULL).set_header(vec![
            Cell::new("Percentile")
                .fg(Color::Cyan)
                .add_attribute(Attribute::Bold),
            Cell::new("Latency (ms)")
                .fg(Color::Cyan)
                .add_attribute(Attribute::Bold),
        ]);

        lat_table.add_row(vec![
            Cell::new("Min"),
            Cell::new(format!("{:.3} ms", self.latency.min_ms)),
        ]);
        lat_table.add_row(vec![
            Cell::new("50% (Median)"),
            Cell::new(format!("{:.3} ms", self.latency.p50_ms)),
        ]);
        lat_table.add_row(vec![
            Cell::new("75%"),
            Cell::new(format!("{:.3} ms", self.latency.p75_ms)),
        ]);
        lat_table.add_row(vec![
            Cell::new("90%"),
            Cell::new(format!("{:.3} ms", self.latency.p90_ms)),
        ]);
        lat_table.add_row(vec![
            Cell::new("95%"),
            Cell::new(format!("{:.3} ms", self.latency.p95_ms)),
        ]);
        lat_table.add_row(vec![
            Cell::new("99%"),
            Cell::new(format!("{:.3} ms", self.latency.p99_ms)).fg(Color::Yellow),
        ]);
        lat_table.add_row(vec![
            Cell::new("99.9%"),
            Cell::new(format!("{:.3} ms", self.latency.p99_9_ms)).fg(Color::Yellow),
        ]);
        lat_table.add_row(vec![
            Cell::new("Max"),
            Cell::new(format!("{:.3} ms", self.latency.max_ms)).fg(Color::Red),
        ]);
        lat_table.add_row(vec![
            Cell::new("Mean ± StdDev"),
            Cell::new(format!(
                "{:.3} ms ± {:.3} ms",
                self.latency.mean_ms, self.latency.stddev_ms
            )),
        ]);

        println!("{lat_table}");
        println!();

        if !self.status_distribution.is_empty() {
            println!("{}", "📊 HTTP Status Codes:".bold().cyan());
            let mut status_table = Table::new();
            status_table.load_preset(UTF8_FULL).set_header(vec![
                Cell::new("Status Code")
                    .fg(Color::Cyan)
                    .add_attribute(Attribute::Bold),
                Cell::new("Count")
                    .fg(Color::Cyan)
                    .add_attribute(Attribute::Bold),
                Cell::new("Percentage")
                    .fg(Color::Cyan)
                    .add_attribute(Attribute::Bold),
            ]);

            let mut sorted_statuses: Vec<_> = self.status_distribution.iter().collect();
            sorted_statuses.sort_by_key(|(code, _)| *code);

            for (&code, &count) in sorted_statuses {
                let pct = if self.total_requests > 0 {
                    (count as f64 / self.total_requests as f64) * 100.0
                } else {
                    0.0
                };

                let color = match code {
                    200..=299 => Color::Green,
                    300..=399 => Color::Blue,
                    400..=499 => Color::Yellow,
                    _ => Color::Red,
                };

                status_table.add_row(vec![
                    Cell::new(code.to_string()).fg(color),
                    Cell::new(count.to_string()),
                    Cell::new(format!("{:.2}%", pct)),
                ]);
            }

            println!("{status_table}");
            println!();
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

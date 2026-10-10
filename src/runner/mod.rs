use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Client;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::sleep;

use crate::cli::{CliArgs, HttpVersion};
use crate::stats::{BenchmarkReport, RequestMetric, WorkerStats};

pub struct BenchmarkRunner {
    args: CliArgs,
}

impl BenchmarkRunner {
    pub fn new(args: CliArgs) -> Self {
        Self { args }
    }

    pub async fn run(&self) -> Result<BenchmarkReport> {
        let headers = self.build_headers()?;
        let client = self.build_client()?;
        let method = self.args.parsed_method()?;
        let body = self.args.get_body_content()?;
        let target_url = self.args.url.clone();
        let concurrency = self.args.concurrency;
        let test_duration = self.args.duration;
        let rate_limit = self.args.rate_limit;

        let is_running = Arc::new(AtomicBool::new(true));

        // Setup Ctrl+C handler
        let is_running_ctrlc = Arc::clone(&is_running);
        let _ = ctrlc::set_handler(move || {
            is_running_ctrlc.store(false, Ordering::SeqCst);
        });

        // Channel for collecting metrics from workers
        let (tx, mut rx) = mpsc::channel::<RequestMetric>(concurrency * 100);

        // Progress bar setup
        let pb = ProgressBar::new(test_duration.as_secs());
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}s ({msg})")
                .unwrap()
                .progress_chars("━╸ "),
        );
        pb.set_message("Benchmarking in progress...");

        let start_time = Instant::now();

        // Spawn metric aggregator task
        let aggregator_handle = tokio::spawn(async move {
            let mut combined_stats = WorkerStats::new();
            while let Some(metric) = rx.recv().await {
                combined_stats.record(metric);
            }
            combined_stats
        });

        // Calculate rate limiter interval if rate limit is specified
        let rate_interval = (1_000_000 * concurrency as u64)
            .checked_div(rate_limit)
            .map(|micros| Duration::from_micros(micros.max(1)));

        // Spawn worker tasks
        let mut worker_handles = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            let client = client.clone();
            let target_url = target_url.clone();
            let method = method.clone();
            let headers = headers.clone();
            let body = body.clone();
            let tx = tx.clone();
            let is_running = Arc::clone(&is_running);

            let handle = tokio::spawn(async move {
                while is_running.load(Ordering::Relaxed) {
                    let req_start = Instant::now();
                    let mut req = client.request(method.clone(), target_url.clone());
                    req = req.headers(headers.clone());

                    if let Some(ref data) = body {
                        req = req.body(data.clone());
                    }

                    match req.send().await {
                        Ok(res) => {
                            let status = res.status().as_u16();
                            let is_error = res.error_for_status_ref().is_err();

                            // Read body bytes to measure transfer and avoid keeping connections blocked
                            let body_bytes = match res.bytes().await {
                                Ok(b) => b.len(),
                                Err(_) => 0,
                            };

                            let latency = req_start.elapsed();
                            let metric = RequestMetric {
                                latency,
                                status: Some(status),
                                bytes: body_bytes,
                                is_error,
                            };

                            if tx.send(metric).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => {
                            let latency = req_start.elapsed();
                            let metric = RequestMetric {
                                latency,
                                status: None,
                                bytes: 0,
                                is_error: true,
                            };

                            if tx.send(metric).await.is_err() {
                                break;
                            }
                        }
                    }

                    if let Some(interval) = rate_interval {
                        sleep(interval).await;
                    }
                }
            });

            worker_handles.push(handle);
        }

        // Drop initial tx handle so receiver can finish when workers are done
        drop(tx);

        // Timer loop to update progress bar and stop after duration
        let end_deadline = start_time + test_duration;
        while Instant::now() < end_deadline && is_running.load(Ordering::Relaxed) {
            let elapsed_secs = start_time.elapsed().as_secs();
            pb.set_position(elapsed_secs.min(test_duration.as_secs()));
            sleep(Duration::from_millis(200)).await;
        }

        is_running.store(false, Ordering::SeqCst);
        pb.finish_with_message("Done!");

        // Wait for workers to shut down gracefully
        for handle in worker_handles {
            let _ = handle.await;
        }

        // Wait for aggregator to complete
        let final_stats = aggregator_handle.await.context("Aggregator task failed")?;
        let actual_duration = start_time.elapsed();

        let report = BenchmarkReport::from_stats(
            final_stats,
            actual_duration,
            self.args.url.to_string(),
            self.args.concurrency,
        );

        Ok(report)
    }

    fn build_headers(&self) -> Result<HeaderMap> {
        let mut header_map = HeaderMap::new();
        header_map.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_static("gale/0.1.0 (Rust modern benchmark tool)"),
        );

        for header_str in &self.args.headers {
            let parts: Vec<&str> = header_str.splitn(2, ':').collect();
            if parts.len() == 2 {
                let name = HeaderName::from_str(parts[0].trim())
                    .context(format!("Invalid header name: {}", parts[0]))?;
                let val = HeaderValue::from_str(parts[1].trim())
                    .context(format!("Invalid header value: {}", parts[1]))?;
                header_map.insert(name, val);
            } else {
                anyhow::bail!(
                    "Invalid header format '{}'. Expected 'Key: Value'",
                    header_str
                );
            }
        }

        Ok(header_map)
    }

    fn build_client(&self) -> Result<Client> {
        let mut builder = Client::builder()
            .timeout(self.args.timeout)
            .danger_accept_invalid_certs(self.args.insecure)
            .pool_max_idle_per_host(self.args.concurrency * 2)
            .tcp_nodelay(true);

        if self.args.http_version == HttpVersion::Http2 {
            builder = builder.http2_prior_knowledge();
        }

        builder.build().context("Failed to initialize HTTP client")
    }
}

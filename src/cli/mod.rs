use clap::{Parser, ValueEnum};
use reqwest::Method;
use std::time::Duration;
use url::Url;

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum HttpVersion {
    Http1,
    Http2,
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "gale",
    author,
    version,
    about = "🌪️ Fast, modern HTTP/1.1 & HTTP/2 load testing and benchmarking tool",
    long_about = "Gale is a high-performance HTTP benchmark tool designed for low latency overhead, accurate percentile tracking (HDR Histogram), and rich reporting."
)]
pub struct CliArgs {
    /// Target URL to benchmark
    #[arg(value_name = "URL")]
    pub url: Url,

    /// Number of concurrent client connections/workers
    #[arg(short = 'c', long = "concurrency", default_value_t = 50)]
    pub concurrency: usize,

    /// Total duration of the test (e.g. 10s, 1m, 30s)
    #[arg(short = 'd', long = "duration", default_value = "10s", value_parser = humantime::parse_duration)]
    pub duration: Duration,

    /// Target rate limit in requests per second (0 for uncapped maximum speed)
    #[arg(short = 'r', long = "rate", default_value_t = 0)]
    pub rate_limit: u64,

    /// HTTP method to use (GET, POST, PUT, DELETE, PATCH, HEAD)
    #[arg(short = 'm', long = "method", default_value = "GET")]
    pub method: String,

    /// HTTP headers in 'Key: Value' format (can be specified multiple times)
    #[arg(short = 'H', long = "header")]
    pub headers: Vec<String>,

    /// Request body payload as string
    #[arg(short = 'b', long = "body")]
    pub body: Option<String>,

    /// Request body read from file
    #[arg(long = "body-file")]
    pub body_file: Option<std::path::PathBuf>,

    /// Request timeout (e.g. 3s, 500ms)
    #[arg(short = 't', long = "timeout", default_value = "5s", value_parser = humantime::parse_duration)]
    pub timeout: Duration,

    /// HTTP protocol version preference (http1, http2)
    #[arg(long = "http", value_enum, default_value_t = HttpVersion::Http1)]
    pub http_version: HttpVersion,

    /// Disable TLS certificate verification (insecure)
    #[arg(short = 'k', long = "insecure", default_value_t = false)]
    pub insecure: bool,

    /// Export benchmark results as JSON to file or stdout
    #[arg(long = "json")]
    pub json_output: Option<Option<std::path::PathBuf>>,
}

impl CliArgs {
    pub fn parsed_method(&self) -> anyhow::Result<Method> {
        self.method
            .to_uppercase()
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid HTTP method: {}", self.method))
    }

    pub fn get_body_content(&self) -> anyhow::Result<Option<Vec<u8>>> {
        if let Some(ref text) = self.body {
            Ok(Some(text.as_bytes().to_vec()))
        } else if let Some(ref path) = self.body_file {
            let bytes = std::fs::read(path)?;
            Ok(Some(bytes))
        } else {
            Ok(None)
        }
    }
}

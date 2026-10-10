<div align="center">

# 🌪️ Gale

**Modern, ultra-fast HTTP/1.1 & HTTP/2 load generator and benchmarking tool written in Rust.**

[![CI](https://github.com/Sunfalll-alt/gale/actions/workflows/ci.yml/badge.svg)](https://github.com/Sunfalll-alt/gale/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](#license)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)

</div>

---

## ⚡ Highlights

- 🚀 **High Throughput & Low Overhead**: Built on top of `tokio` and `reqwest` connection pooling with `TCP_NODELAY`.
- 📊 **High Dynamic Range (HDR) Latency Tracking**: Records microsecond-accurate percentiles (`p50`, `p75`, `p90`, `p95`, `p99`, `p99.9`, `min`, `max`, `stddev`) using `hdrhistogram`.
- 🌐 **HTTP/1.1 & HTTP/2 Support**: Benchmark standard endpoints or force HTTP/2 Prior Knowledge connections.
- 🎯 **Rate Limiting**: Cap requests per second (RPS) to run controlled throughput experiments without overwhelming target servers.
- 🖥️ **Interactive Terminal UI (TUI) Mode**: Live interactive dashboard with real-time RPS trend graph and iconic The Binding of Isaac Rebirth item art (**Breath of Life**) via `--tui`.
- 🎨 **Rich Terminal Visuals**: Real-time progress bar, colored summary tables powered by `comfy-table`.
- 💾 **Machine-Readable JSON Output**: Export full benchmark reports to JSON for CI/CD assertions or visualization dashboards.
- 🛑 **Graceful Shutdown**: Intercepts `Ctrl+C` and compiles benchmark reports on partial runs without losing collected data.

---

## 📦 Installation

### From Source

Ensure you have Rust installed (`cargo` >= 1.75):

```bash
git clone https://github.com/Sunfalll-alt/gale.git
cd gale
cargo install --path .
```

Or build an optimized release binary:

```bash
cargo build --release
# Executable located at ./target/release/gale
```

---

## 🚀 Quick Start

### Basic Benchmark
Run a 10-second benchmark against an endpoint with 50 concurrent connections:
```bash
gale http://127.0.0.1:8080/api/v1/ping
```

### Interactive TUI Dashboard (with Breath of Life ASCII Art)
Run with live terminal charts and Isaac's Breath of Life item card:
```bash
gale http://127.0.0.1:8080/api/v1/ping -c 50 -d 30s --tui
```

### High Concurrency & Custom Duration
Run with 200 workers for 30 seconds:
```bash
gale http://127.0.0.1:8080/api/users -c 200 -d 30s
```

### Rate Limiting (Capped RPS)
Run at a target rate of 1,000 requests per second:
```bash
gale http://127.0.0.1:8080/api/orders -c 20 -d 15s -r 1000
```

### POST Requests with Headers & Body
```bash
gale http://127.0.0.1:8080/api/items \
  -m POST \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer mytoken" \
  -b '{"name":"item_01","price":100}' \
  -c 30 \
  -d 10s
```

### JSON Export for CI/CD Pipelines
```bash
gale http://127.0.0.1:8080/health -c 10 -d 5s --json report.json
```

---

## 🖥️ Example Terminal Output

```text
🌪️ Gale - Modern HTTP Load Generator & Benchmarker
Running 10s test @ http://127.0.0.1:8080/api with 50 workers (rate limit: unlimited)
[00:00:10] [━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━] 10/10s (Done!)

⚡ Benchmark Summary:
┌─────────────────────────┬──────────────────────┐
│ Target URL              │ http://127.0.0.1:8080 │
│ Concurrency             │ 50                   │
│ Duration                │ 10.00 s              │
│ Total Requests          │ 48291                │
│ Throughput (RPS)        │ 4829.10 req/s        │
│ Transfer Rate           │ 1.25 MB/s            │
│ Total Data Transferred  │ 12.50 MB             │
│ Success Rate            │ 100.00% (0 errors)   │
└─────────────────────────┴──────────────────────┘

⏱️ Latency Distribution:
┌─────────────────┬──────────────┐
│ Percentile      │ Latency (ms) │
├─────────────────┼──────────────┤
│ Min             │ 0.125 ms     │
│ 50% (Median)    │ 8.420 ms     │
│ 75%             │ 11.230 ms    │
│ 90%             │ 14.890 ms    │
│ 95%             │ 18.210 ms    │
│ 99%             │ 24.310 ms    │
│ 99.9%           │ 32.180 ms    │
│ Max             │ 45.120 ms    │
│ Mean ± StdDev   │ 9.150 ms ± 3.420 ms │
└─────────────────┴──────────────┘

📊 HTTP Status Codes:
┌─────────────┬───────┬────────────┐
│ Status Code │ Count │ Percentage │
├─────────────┼───────┼────────────┤
│ 200         │ 48291 │ 100.00%    │
└─────────────┴───────┴────────────┘
```

---

## ⚙️ CLI Reference

```text
Usage: gale [OPTIONS] <URL>

Arguments:
  <URL>  Target URL to benchmark

Options:
  -c, --concurrency <CONCURRENCY>  Number of concurrent client connections/workers [default: 50]
  -d, --duration <DURATION>        Total duration of the test (e.g. 10s, 1m, 30s) [default: 10s]
  -r, --rate <RATE>                Target rate limit in requests per second (0 for uncapped) [default: 0]
  -m, --method <METHOD>            HTTP method (GET, POST, PUT, DELETE, etc.) [default: GET]
  -H, --header <HEADER>            HTTP headers in 'Key: Value' format
  -b, --body <BODY>                Request body payload as string
      --body-file <PATH>           Request body payload read from file
  -t, --timeout <TIMEOUT>          Request timeout (e.g. 5s, 500ms) [default: 5s]
      --http <HTTP>                HTTP version preference (http1, http2) [default: http1]
  -k, --insecure                   Disable TLS certificate verification
      --json [<PATH>]              Export benchmark results as JSON to file or stdout
  -h, --help                       Print help
  -V, --version                    Print version
```

---

## 🧪 Testing

Run test suite including mock-server integration tests:

```bash
cargo test
```

Run clippy verification:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

---

## 📜 License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

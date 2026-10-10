use clap::Parser;
use gale::cli::{CliArgs, HttpVersion};
use gale::runner::BenchmarkRunner;
use gale::stats::{RequestMetric, WorkerStats};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn test_worker_stats_recording() {
    let mut stats = WorkerStats::new();
    assert_eq!(stats.total_requests, 0);

    stats.record(RequestMetric {
        latency: Duration::from_millis(15),
        status: Some(200),
        bytes: 128,
        is_error: false,
    });

    stats.record(RequestMetric {
        latency: Duration::from_millis(50),
        status: Some(404),
        bytes: 64,
        is_error: true,
    });

    assert_eq!(stats.total_requests, 2);
    assert_eq!(stats.total_bytes, 192);
    assert_eq!(stats.errors, 1);
    assert_eq!(stats.status_codes.get(&200), Some(&1));
    assert_eq!(stats.status_codes.get(&404), Some(&1));
}

#[test]
fn test_cli_parsing_defaults() {
    let args = CliArgs::parse_from(["gale", "http://127.0.0.1:8080/test"]);
    assert_eq!(
        args.url.as_ref().unwrap().as_str(),
        "http://127.0.0.1:8080/test"
    );
    assert_eq!(args.concurrency, 50);
    assert_eq!(args.duration, Duration::from_secs(10));
    assert_eq!(args.method, "GET");
    assert_eq!(args.http_version, HttpVersion::Http1);
    assert!(!args.insecure);
}

#[tokio::test]
async fn test_benchmark_against_mock_server() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/benchmark"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello from gale test"))
        .mount(&mock_server)
        .await;

    let target_url = format!("{}/benchmark", mock_server.uri());
    let args = CliArgs::parse_from(["gale", &target_url, "-c", "5", "-d", "1s", "-t", "2s"]);

    let runner = BenchmarkRunner::new(args);
    let report = runner.run().await.expect("Runner execution should succeed");

    assert!(report.total_requests > 0);
    assert_eq!(report.error_requests, 0);
    assert_eq!(report.concurrency, 5);
    assert!(report.status_distribution.contains_key(&200));
}

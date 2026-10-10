mod cli;
mod runner;
mod stats;
mod tui;

use clap::Parser;
use colored::*;
use std::fs::File;
use std::io::Write;

use cli::CliArgs;
use runner::BenchmarkRunner;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = CliArgs::parse();

    // If no URL passed (e.g. user just ran `gale`), launch interactive TUI menu
    if args.url.is_none() {
        match tui::menu::run_interactive_menu()? {
            Some(configured_args) => {
                args = configured_args;
            }
            None => {
                println!("{}", "Exited Gale menu.".yellow());
                return Ok(());
            }
        }
    }

    let target_url = args.url.as_ref().unwrap();

    if !args.tui {
        println!(
            "{}",
            "🌪️ Gale - Modern HTTP Load Generator & Benchmarker"
                .bold()
                .cyan()
        );
        println!(
            "Running {}s test @ {} with {} workers (rate limit: {})",
            args.duration.as_secs(),
            target_url.as_str().yellow(),
            args.concurrency.to_string().green(),
            if args.rate_limit > 0 {
                format!("{} req/s", args.rate_limit)
            } else {
                "unlimited".to_string()
            }
        );
    }

    let json_opt = args.json_output.clone();
    let runner = BenchmarkRunner::new(args);
    let report = runner.run().await?;

    // Print styled terminal report
    report.print_terminal();

    // Export to JSON if requested
    if let Some(path_opt) = json_opt {
        let json_str = serde_json::to_string_pretty(&report)?;
        if let Some(path) = path_opt {
            let mut file = File::create(&path)?;
            file.write_all(json_str.as_bytes())?;
            println!(
                "💾 Results exported to {}",
                path.display().to_string().cyan()
            );
        } else {
            println!("\n{}", json_str);
        }
    }

    Ok(())
}

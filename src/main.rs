mod config;
mod display;
mod format;
mod github;
mod web;

use std::future::Future;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};

use config::Config;
use github::{Client, StatsCalculator, UserStats};

#[tokio::main]
async fn main() -> ExitCode {
    let cfg = match Config::load() {
        Ok(cfg) => cfg,
        Err(err) => {
            display::error(&format!("Configuration error: {err:#}"));
            return ExitCode::FAILURE;
        }
    };

    // Ctrl+C or SIGTERM while fetching aborts the run.
    let stats = tokio::select! {
        stats = collect_stats(&cfg) => match stats {
            Some(stats) => stats,
            None => return ExitCode::FAILURE,
        },
        () = shutdown_signal() => {
            display::error("Interrupted");
            return ExitCode::FAILURE;
        }
    };

    if cfg.web {
        let server = match web::Server::new(&stats, "127.0.0.1", cfg.port).await {
            Ok(server) => server,
            Err(err) => {
                display::error(&format!("Failed to start web server: {err:#}"));
                return ExitCode::FAILURE;
            }
        };

        display::success(&format!(
            "Serving statistics at {} (press Ctrl+C to stop)",
            server.url()
        ));

        if let Err(err) = server.run(shutdown_signal()).await {
            display::error(&format!("Web server error: {err:#}"));
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    if let Err(err) = display::print_stats(&stats, cfg.format) {
        display::error(&format!("Failed to display statistics: {err:#}"));
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

/// Fetches and calculates the statistics, reporting progress and problems
/// as it goes. Returns `None` after reporting a fatal error.
async fn collect_stats(cfg: &Config) -> Option<UserStats> {
    let client = match Client::new(&cfg.token, cfg.max_workers) {
        Ok(client) => client,
        Err(err) => {
            display::error(&format!("Configuration error: {err:#}"));
            return None;
        }
    };

    let username = match cfg.username.clone() {
        Some(username) => username,
        None => {
            let result =
                with_spinner("Getting authenticated user...", client.authenticated_user()).await;
            match result {
                Ok(username) => {
                    display::success(&format!("Authenticated as: {username}"));
                    username
                }
                Err(err) => {
                    display::error(&format!("Failed to get authenticated user: {err:#}"));
                    return None;
                }
            }
        }
    };

    if let Err(err) = check_rate_limit(&client).await {
        display::warning(&format!("Rate limit check failed: {err:#}"));
    }

    println!();
    display::heading("🚀 Fetching GitHub statistics...");
    println!();

    let mut calculator = StatsCalculator::new(&client);
    let result = with_spinner(
        "Analyzing profile and repositories...",
        calculator.calculate(&username, cfg.full_scan),
    )
    .await;

    let stats = match result {
        Ok(stats) => stats,
        Err(err) => {
            display::error(&format!("Failed to calculate statistics: {err:#}"));
            return None;
        }
    };

    for warning in calculator.warnings() {
        display::warning(warning);
    }
    display::success("Statistics calculated successfully");

    Some(stats)
}

async fn check_rate_limit(client: &Client) -> Result<()> {
    if let Some(rate) = client.rate_limit().await? {
        if rate.remaining < 100 {
            display::warning(&format!(
                "API Rate Limit: {}/{} remaining (resets at {})",
                rate.remaining,
                rate.limit,
                rate.reset.format("%H:%M:%S")
            ));
        } else {
            display::success(&format!(
                "API Rate Limit: {}/{} remaining",
                rate.remaining, rate.limit
            ));
        }
    }
    Ok(())
}

/// Clears the spinner line even when the future it wraps is cancelled.
struct Spinner(ProgressBar);

impl Drop for Spinner {
    fn drop(&mut self) {
        self.0.finish_and_clear();
    }
}

/// Shows a spinner with `message` while `fut` runs.
async fn with_spinner<T>(message: &'static str, fut: impl Future<Output = T>) -> T {
    let spinner = Spinner(ProgressBar::new_spinner());
    let spinner = &spinner.0;
    spinner.set_style(
        ProgressStyle::with_template("{spinner} {msg}")
            .expect("valid template")
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
    );
    spinner.set_message(message);
    spinner.enable_steady_tick(Duration::from_millis(100));

    fut.await
}

/// Completes on Ctrl+C or, on Unix, SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

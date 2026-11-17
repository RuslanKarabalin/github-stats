use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Table,
    Json,
}

/// A CLI tool to display GitHub profile statistics.
#[derive(Debug, Parser)]
#[command(
    name = "github-stats",
    version,
    after_help = "\
Examples:
  github-stats --user octocat
  github-stats --user octocat --full --format json > stats.json
  github-stats --user octocat --web --port 3000
  github-stats --private --web

Authentication:
  Set GITHUB_TOKEN in the environment or in a .env file in the current
  directory. --token also works, but a token on the command line is visible
  to other processes and stays in the shell history.
  Create a token at: https://github.com/settings/tokens"
)]
pub struct Config {
    /// GitHub Personal Access Token (overrides GITHUB_TOKEN; prefer the variable)
    #[arg(long, default_value = "", hide_default_value = true)]
    pub token: String,

    /// GitHub username to analyze (defaults to authenticated user)
    #[arg(long = "user")]
    pub username: Option<String>,

    /// Scan the commit history of every repository to find the most active
    /// hour (slow). Without it, recent push events are used
    #[arg(long = "full")]
    pub full_scan: bool,

    /// Include your private repositories (only when analyzing the
    /// authenticated user)
    #[arg(long = "private")]
    pub include_private: bool,

    /// Output format
    #[arg(long, value_enum, default_value_t = Format::Table)]
    pub format: Format,

    /// Maximum concurrent API requests (1-50)
    #[arg(long = "workers", default_value_t = 10)]
    pub max_workers: usize,

    /// Serve the statistics as a page on localhost instead of printing them (ignores --format)
    #[arg(long)]
    pub web: bool,

    /// Port to listen on with --web (0 picks a free port)
    #[arg(long, default_value_t = 8080)]
    pub port: u16,
}

impl Config {
    pub fn load() -> Result<Self> {
        let cfg = Self::parse();
        // Variables already set in the environment win over the .env file.
        match dotenvy::dotenv() {
            Ok(_) => {}
            Err(err) if err.not_found() => {}
            Err(err) => bail!("failed to read .env file: {err}"),
        }
        Self::from_parsed(cfg, std::env::var("GITHUB_TOKEN").ok())
    }

    fn from_parsed(mut cfg: Self, env_token: Option<String>) -> Result<Self> {
        if cfg.token.is_empty() {
            cfg.token = env_token.unwrap_or_default();
        }
        if cfg.token.is_empty() {
            bail!(
                "GitHub token is required. Set GITHUB_TOKEN environment variable or use --token flag"
            );
        }
        if !(1..=50).contains(&cfg.max_workers) {
            bail!("workers must be between 1 and 50");
        }
        cfg.username = cfg.username.filter(|u| !u.is_empty());
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str], env: Option<&str>) -> Result<Config> {
        let cfg =
            Config::try_parse_from(std::iter::once("github-stats").chain(args.iter().copied()))?;
        Config::from_parsed(cfg, env.map(str::to_string))
    }

    #[test]
    fn token_from_flag_overrides_env() {
        let cfg = parse(&["--token", "flag", "--user", "octocat"], Some("env")).unwrap();
        assert_eq!(cfg.token, "flag");
        assert_eq!(cfg.username.as_deref(), Some("octocat"));
        assert_eq!(cfg.format, Format::Table);
        assert_eq!(cfg.port, 8080);
        assert!(!cfg.full_scan && !cfg.include_private);
    }

    #[test]
    fn parses_scan_flags() {
        let cfg = parse(&["--full", "--private"], Some("t")).unwrap();
        assert!(cfg.full_scan && cfg.include_private);
    }

    #[test]
    fn token_from_env() {
        assert_eq!(parse(&[], Some("env")).unwrap().token, "env");
        assert!(parse(&[], None).is_err());
    }

    #[test]
    fn validates_workers() {
        assert!(parse(&["--workers", "0"], Some("t")).is_err());
        assert!(parse(&["--workers", "51"], Some("t")).is_err());
        assert!(parse(&["--workers", "50", "--format", "json"], Some("t")).is_ok());
    }
}

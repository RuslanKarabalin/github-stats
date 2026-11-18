use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Table,
    Json,
}

#[derive(Debug, Parser)]
#[command(
    name = "github-stats",
    version,
    about = "A CLI tool to display GitHub profile statistics",
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
    #[arg(
        long,
        default_value = "",
        hide_default_value = true,
        help = "GitHub Personal Access Token (overrides GITHUB_TOKEN; prefer the variable)"
    )]
    pub token: String,

    #[arg(
        long = "user",
        help = "GitHub username to analyze (defaults to authenticated user)"
    )]
    pub username: Option<String>,

    #[arg(
        long = "full",
        help = "Scan the commit history of every repository to find the most active hour (slow). Without it, recent push events are used"
    )]
    pub full_scan: bool,

    #[arg(
        long = "private",
        help = "Include your private repositories (only when analyzing the authenticated user)"
    )]
    pub include_private: bool,

    #[arg(long, value_enum, default_value_t = Format::Table, help = "Output format")]
    pub format: Format,

    #[arg(
        long = "workers",
        default_value_t = 10,
        help = "Maximum concurrent API requests (1-50)"
    )]
    pub max_workers: usize,

    #[arg(
        long,
        help = "Serve the statistics as a page on localhost instead of printing them (ignores --format)"
    )]
    pub web: bool,

    #[arg(
        long,
        default_value_t = 8080,
        help = "Port to listen on with --web (0 picks a free port)"
    )]
    pub port: u16,
}

impl Config {
    pub fn load() -> Result<Self> {
        let cfg = Self::parse();
        match dotenvy::from_path(".env") {
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
        if let Some(username) = &cfg.username
            && !is_valid_username(username)
        {
            bail!(
                "invalid GitHub username '{username}': use up to 39 letters, digits \
                 or hyphens, not starting with a hyphen"
            );
        }
        Ok(cfg)
    }
}

pub fn is_valid_username(name: &str) -> bool {
    (1..=39).contains(&name.len())
        && !name.starts_with('-')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
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
    fn validates_username() {
        assert!(parse(&["--user", "octo-cat42"], Some("t")).is_ok());
        assert_eq!(parse(&["--user", ""], Some("t")).unwrap().username, None);
        for bad in ["-octocat", "octo/cat", "octo cat", "a?b", &"a".repeat(40)] {
            assert!(parse(&["--user", bad], Some("t")).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn validates_workers() {
        assert!(parse(&["--workers", "0"], Some("t")).is_err());
        assert!(parse(&["--workers", "51"], Some("t")).is_err());
        assert!(parse(&["--workers", "50", "--format", "json"], Some("t")).is_ok());
    }
}

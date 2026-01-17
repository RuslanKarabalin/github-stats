package config

import (
	"flag"
	"fmt"
	"os"
	"strings"
)

type Config struct {
	Token      string
	Username   string
	FullScan   bool
	NoCache    bool
	Format     string
	StatsOnly  []string
	MaxWorkers int
}

func Load() (*Config, error) {
	cfg := &Config{}

	flag.StringVar(&cfg.Token, "token", "", "GitHub Personal Access Token (overrides GITHUB_TOKEN env)")
	flag.StringVar(&cfg.Username, "user", "", "GitHub username to analyze (defaults to authenticated user)")
	flag.BoolVar(&cfg.FullScan, "full", false, "Perform full history scan (slower but complete)")
	flag.BoolVar(&cfg.NoCache, "no-cache", false, "Disable caching")
	flag.StringVar(&cfg.Format, "format", "table", "Output format: table, json")
	statsOnly := flag.String("stats", "", "Comma-separated stats to show: profile,repos,streak,languages,prs,issues,reviews (default: all)")
	flag.IntVar(&cfg.MaxWorkers, "workers", 10, "Maximum concurrent API requests")

	flag.Usage = func() {
		fmt.Fprintf(os.Stderr, "Usage: github-stats [options]\n\n")
		fmt.Fprintf(os.Stderr, "A CLI tool to display GitHub profile statistics.\n\n")
		fmt.Fprintf(os.Stderr, "Options:\n")
		flag.PrintDefaults()
		fmt.Fprintf(os.Stderr, "\nExamples:\n")
		fmt.Fprintf(os.Stderr, "  github-stats --user octocat\n")
		fmt.Fprintf(os.Stderr, "  github-stats --user octocat --full --format json\n")
		fmt.Fprintf(os.Stderr, "  github-stats --token ghp_xxx --user octocat\n")
		fmt.Fprintf(os.Stderr, "\nAuthentication:\n")
		fmt.Fprintf(os.Stderr, "  Set GITHUB_TOKEN environment variable or use --token flag\n")
		fmt.Fprintf(os.Stderr, "  Create token at: https://github.com/settings/tokens\n")
	}

	flag.Parse()

	if *statsOnly != "" {
		cfg.StatsOnly = strings.Split(*statsOnly, ",")
		for i, s := range cfg.StatsOnly {
			cfg.StatsOnly[i] = strings.TrimSpace(s)
		}
	}

	if cfg.Token == "" {
		cfg.Token = os.Getenv("GITHUB_TOKEN")
	}

	if cfg.Token == "" {
		return nil, fmt.Errorf("GitHub token is required. Set GITHUB_TOKEN environment variable or use --token flag")
	}

	if cfg.Format != "table" && cfg.Format != "json" {
		return nil, fmt.Errorf("invalid format: %s (must be 'table' or 'json')", cfg.Format)
	}

	if cfg.MaxWorkers < 1 || cfg.MaxWorkers > 50 {
		return nil, fmt.Errorf("workers must be between 1 and 50")
	}

	return cfg, nil
}

func (c *Config) ShouldShowStat(stat string) bool {
	if len(c.StatsOnly) == 0 {
		return true
	}
	for _, s := range c.StatsOnly {
		if s == stat {
			return true
		}
	}
	return false
}

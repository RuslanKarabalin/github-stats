package main

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github-stats/internal/config"
	"github-stats/internal/display"
	"github-stats/internal/github"
	"github-stats/internal/web"

	"github.com/briandowns/spinner"
	"github.com/fatih/color"
)

func main() {
	cfg, err := config.Load()
	if err != nil {
		display.DisplayError(fmt.Sprintf("Configuration error: %v", err))
		os.Exit(1)
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	client := github.NewClient(ctx, cfg.Token, cfg.MaxWorkers)

	username := cfg.Username
	if username == "" {
		s := spinner.New(spinner.CharSets[14], 100*time.Millisecond)
		s.Suffix = " Getting authenticated user..."
		s.Start()

		username, err = client.GetAuthenticatedUser()
		s.Stop()

		if err != nil {
			display.DisplayError(fmt.Sprintf("Failed to get authenticated user: %v", err))
			os.Exit(1)
		}
		display.DisplaySuccess(fmt.Sprintf("Authenticated as: %s", username))
	}

	if err := checkRateLimit(client); err != nil {
		display.DisplayWarning(fmt.Sprintf("Rate limit check failed: %v", err))
	}

	statsCalc := github.NewStatsCalculator(client)

	cyan := color.New(color.FgCyan, color.Bold)
	fmt.Println()
	_, _ = cyan.Println("🚀 Fetching GitHub statistics...")
	fmt.Println()

	s := spinner.New(spinner.CharSets[14], 100*time.Millisecond)
	s.Suffix = " Analyzing profile and repositories..."
	s.Start()

	stats, err := statsCalc.Calculate(ctx, username, cfg.FullScan)
	s.Stop()

	if err != nil {
		display.DisplayError(fmt.Sprintf("Failed to calculate statistics: %v", err))
		os.Exit(1)
	}

	for _, warning := range statsCalc.Warnings() {
		display.DisplayWarning(warning)
	}

	display.DisplaySuccess("Statistics calculated successfully")

	if cfg.Web {
		server, err := web.New(stats, "127.0.0.1", cfg.Port)
		if err != nil {
			display.DisplayError(fmt.Sprintf("Failed to start web server: %v", err))
			os.Exit(1)
		}

		display.DisplaySuccess(fmt.Sprintf("Serving statistics at %s (press Ctrl+C to stop)", server.URL()))

		if err := server.Run(ctx); err != nil {
			display.DisplayError(fmt.Sprintf("Web server error: %v", err))
			os.Exit(1)
		}
		return
	}

	formatter := display.NewFormatter(cfg.Format)
	if err := formatter.Display(stats); err != nil {
		display.DisplayError(fmt.Sprintf("Failed to display statistics: %v", err))
		os.Exit(1)
	}
}

func checkRateLimit(client *github.Client) error {
	limits, err := client.CheckRateLimit()
	if err != nil {
		return err
	}

	if limits.Core != nil {
		remaining := limits.Core.Remaining
		limit := limits.Core.Limit
		reset := limits.Core.Reset.Time

		if remaining < 100 {
			display.DisplayWarning(fmt.Sprintf(
				"API Rate Limit: %d/%d remaining (resets at %s)",
				remaining, limit, reset.Format("15:04:05"),
			))
		} else {
			display.DisplaySuccess(fmt.Sprintf(
				"API Rate Limit: %d/%d remaining",
				remaining, limit,
			))
		}
	}

	return nil
}

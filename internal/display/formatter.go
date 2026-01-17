package display

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"time"

	"github-stats/internal/github"

	"github.com/fatih/color"
	"github.com/olekukonko/tablewriter"
	"github.com/olekukonko/tablewriter/tw"
)

type Formatter struct {
	format string
}

func NewFormatter(format string) *Formatter {
	return &Formatter{format: format}
}

func (f *Formatter) Display(stats *github.UserStats) error {
	switch f.format {
	case "json":
		return f.displayJSON(stats)
	case "table":
		return f.displayTable(stats)
	default:
		return fmt.Errorf("unsupported format: %s", f.format)
	}
}

func (f *Formatter) displayJSON(stats *github.UserStats) error {
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	return encoder.Encode(stats)
}

func (f *Formatter) displayTable(stats *github.UserStats) error {
	cyan := color.New(color.FgCyan, color.Bold)
	green := color.New(color.FgGreen)
	blue := color.New(color.FgBlue)

	_, _ = cyan.Println("\n" + strings.Repeat("=", 80))
	_, _ = cyan.Printf("  GitHub Statistics for @%s\n", stats.Username)
	_, _ = cyan.Println(strings.Repeat("=", 80))

	fmt.Println()
	_, _ = green.Println("👤 PROFILE")
	fmt.Println(strings.Repeat("-", 80))

	table := tablewriter.NewWriter(os.Stdout)
	table.Header("Field", "Value")
	table.Options(
		tablewriter.WithAlignment(tw.MakeAlign(2, tw.AlignLeft)),
	)

	if stats.Name != "" {
		_ = table.Append([]string{"Name", stats.Name})
	}
	_ = table.Append([]string{"Username", stats.Username})
	if stats.Bio != "" {
		_ = table.Append([]string{"Bio", truncate(stats.Bio, 60)})
	}
	if stats.Company != "" {
		_ = table.Append([]string{"Company", stats.Company})
	}
	if stats.Location != "" {
		_ = table.Append([]string{"Location", stats.Location})
	}
	if stats.Blog != "" {
		_ = table.Append([]string{"Website", stats.Blog})
	}
	_ = table.Append([]string{"Joined", stats.CreatedAt.Format("January 2, 2006")})
	_ = table.Append([]string{"Account Age", stats.AccountAge.String()})
	_ = table.Append([]string{"Followers", fmt.Sprintf("%d", stats.Followers)})
	_ = table.Append([]string{"Following", fmt.Sprintf("%d", stats.Following)})

	_ = table.Render()

	fmt.Println()
	_, _ = green.Println("📚 REPOSITORY STATISTICS")
	fmt.Println(strings.Repeat("-", 80))

	table = tablewriter.NewWriter(os.Stdout)
	table.Header("Metric", "Value")
	table.Options(
		tablewriter.WithAlignment(tw.MakeAlign(2, tw.AlignLeft)),
	)

	_ = table.Append([]string{"Public Repositories", fmt.Sprintf("%d", stats.PublicRepos)})
	_ = table.Append([]string{"Public Gists", fmt.Sprintf("%d", stats.PublicGists)})
	_ = table.Append([]string{"Total Stars Received", fmt.Sprintf("%d ⭐", stats.TotalStars)})
	_ = table.Append([]string{"Total Forks Received", fmt.Sprintf("%d", stats.TotalForks)})

	_ = table.Render()

	fmt.Println()
	_, _ = green.Println("🔥 COMMIT STREAKS")
	fmt.Println(strings.Repeat("-", 80))

	table = tablewriter.NewWriter(os.Stdout)
	table.Header("Metric", "Value")
	table.Options(
		tablewriter.WithAlignment(tw.MakeAlign(2, tw.AlignLeft)),
	)

	if stats.CurrentStreak > 0 {
		_ = table.Append([]string{"Current Streak", fmt.Sprintf("%d days 🔥", stats.CurrentStreak)})
		_ = table.Append([]string{"Current Streak Start", stats.CurrentStreakStart.Format("Jan 2, 2006")})
	} else {
		_ = table.Append([]string{"Current Streak", "0 days (inactive)"})
	}

	_ = table.Append([]string{"Maximum Streak", fmt.Sprintf("%d days 🏆", stats.MaxStreak)})
	if !stats.MaxStreakStart.IsZero() {
		streakRange := fmt.Sprintf("%s - %s",
			stats.MaxStreakStart.Format("Jan 2, 2006"),
			stats.MaxStreakEnd.Format("Jan 2, 2006"))
		_ = table.Append([]string{"Max Streak Period", streakRange})
	}
	_ = table.Append([]string{"Total Commit Days", fmt.Sprintf("%d", stats.TotalCommitDays)})

	_ = table.Render()

	if stats.MostActiveDay != "" || stats.MostActiveHour > 0 {
		fmt.Println()
		_, _ = green.Println("📊 ACTIVITY PATTERNS")
		fmt.Println(strings.Repeat("-", 80))

		table = tablewriter.NewWriter(os.Stdout)
		table.Header("Metric", "Value")
		table.Options(
			tablewriter.WithAlignment(tw.MakeAlign(2, tw.AlignLeft)),
		)

		if stats.MostActiveDay != "" {
			_ = table.Append([]string{"Most Active Day", stats.MostActiveDay})
		}
		if stats.MostActiveHour >= 0 {
			hourStr := formatHour(stats.MostActiveHour)
			_ = table.Append([]string{"Most Active Hour", hourStr})
		}

		_ = table.Render()
	}

	if len(stats.Languages) > 0 {
		fmt.Println()
		_, _ = green.Println("💻 LANGUAGE STATISTICS")
		fmt.Println(strings.Repeat("-", 80))

		langStats := github.GetLanguageStats(stats.Languages)

		table = tablewriter.NewWriter(os.Stdout)
		table.Header("Language", "Bytes", "Percentage")
		table.Options(
			tablewriter.WithAlignment(tw.MakeAlign(3, tw.AlignLeft)),
		)

		count := 10
		if len(langStats.TopLanguages) < count {
			count = len(langStats.TopLanguages)
		}

		for i := 0; i < count; i++ {
			lang := langStats.TopLanguages[i]
			_ = table.Append([]string{
				lang.Name,
				formatBytes(lang.Bytes),
				fmt.Sprintf("%.1f%%", lang.Percentage),
			})
		}

		_ = table.Render()
	}

	if len(stats.TopRepositories) > 0 {
		fmt.Println()
		_, _ = green.Println("🌟 TOP REPOSITORIES (by stars)")
		fmt.Println(strings.Repeat("-", 80))

		table = tablewriter.NewWriter(os.Stdout)
		table.Header("Repository", "Stars", "Forks", "Language")
		table.Options(
			tablewriter.WithAlignment(tw.MakeAlign(4, tw.AlignLeft)),
		)

		for _, repo := range stats.TopRepositories {
			lang := repo.Language
			if lang == "" {
				lang = "N/A"
			}
			_ = table.Append([]string{
				repo.Name,
				fmt.Sprintf("%d ⭐", repo.Stars),
				fmt.Sprintf("%d", repo.Forks),
				lang,
			})
		}

		_ = table.Render()
	}

	if stats.PRStats != nil && stats.PRStats.Total > 0 {
		fmt.Println()
		_, _ = green.Println("🔀 PULL REQUEST STATISTICS")
		fmt.Println(strings.Repeat("-", 80))

		table = tablewriter.NewWriter(os.Stdout)
		table.Header("Metric", "Value")
		table.Options(
			tablewriter.WithAlignment(tw.MakeAlign(2, tw.AlignLeft)),
		)

		_ = table.Append([]string{"Total PRs Created", fmt.Sprintf("%d", stats.PRStats.Total)})
		_ = table.Append([]string{"Open", fmt.Sprintf("%d", stats.PRStats.Open)})
		_ = table.Append([]string{"Merged", fmt.Sprintf("%d ✓", stats.PRStats.Merged)})
		_ = table.Append([]string{"Closed (unmerged)", fmt.Sprintf("%d", stats.PRStats.Closed)})
		if stats.PRStats.AvgMergeTime > 0 {
			_ = table.Append([]string{"Avg Time to Merge", formatDuration(stats.PRStats.AvgMergeTime)})
		}

		_ = table.Render()

		if len(stats.PRStats.TopRepos) > 0 {
			fmt.Println()
			fmt.Println("  Top Repositories by PR Count:")
			for _, repo := range stats.PRStats.TopRepos {
				fmt.Printf("    - %s: %d PRs\n", repo.RepoName, repo.Count)
			}
		}
	}

	if stats.IssueStats != nil && stats.IssueStats.Total > 0 {
		fmt.Println()
		_, _ = green.Println("📋 ISSUE STATISTICS")
		fmt.Println(strings.Repeat("-", 80))

		table = tablewriter.NewWriter(os.Stdout)
		table.Header("Metric", "Value")
		table.Options(
			tablewriter.WithAlignment(tw.MakeAlign(2, tw.AlignLeft)),
		)

		_ = table.Append([]string{"Total Issues Created", fmt.Sprintf("%d", stats.IssueStats.Total)})
		_ = table.Append([]string{"Open", fmt.Sprintf("%d", stats.IssueStats.Open)})
		_ = table.Append([]string{"Closed", fmt.Sprintf("%d ✓", stats.IssueStats.Closed)})
		if stats.IssueStats.AvgCloseTime > 0 {
			_ = table.Append([]string{"Avg Time to Close", formatDuration(stats.IssueStats.AvgCloseTime)})
		}

		_ = table.Render()
	}

	if stats.ReviewStats != nil && stats.ReviewStats.Total > 0 {
		fmt.Println()
		_, _ = green.Println("👀 CODE REVIEW STATISTICS")
		fmt.Println(strings.Repeat("-", 80))

		table = tablewriter.NewWriter(os.Stdout)
		table.Header("Metric", "Value")
		table.Options(
			tablewriter.WithAlignment(tw.MakeAlign(2, tw.AlignLeft)),
		)

		_ = table.Append([]string{"Total Reviews", fmt.Sprintf("%d", stats.ReviewStats.Total)})

		_ = table.Render()

		if len(stats.ReviewStats.TopRepos) > 0 {
			fmt.Println()
			fmt.Println("  Top Repositories by Review Count:")
			for _, repo := range stats.ReviewStats.TopRepos {
				fmt.Printf("    - %s: %d reviews\n", repo.RepoName, repo.Count)
			}
		}
	}

	fmt.Println()
	_, _ = blue.Println(strings.Repeat("-", 80))
	_, _ = blue.Printf("Generated at: %s\n", time.Now().Format("2006-01-02 15:04:05 MST"))
	_, _ = blue.Println(strings.Repeat("=", 80))
	fmt.Println()

	return nil
}

func truncate(s string, maxLen int) string {
	if len(s) <= maxLen {
		return s
	}
	return s[:maxLen-3] + "..."
}

func formatBytes(bytes int64) string {
	const unit = 1024
	if bytes < unit {
		return fmt.Sprintf("%d B", bytes)
	}
	div, exp := int64(unit), 0
	for n := bytes / unit; n >= unit; n /= unit {
		div *= unit
		exp++
	}
	return fmt.Sprintf("%.1f %cB", float64(bytes)/float64(div), "KMGTPE"[exp])
}

func formatHour(hour int) string {
	if hour == 0 {
		return "12:00 AM"
	} else if hour < 12 {
		return fmt.Sprintf("%d:00 AM", hour)
	} else if hour == 12 {
		return "12:00 PM"
	} else {
		return fmt.Sprintf("%d:00 PM", hour-12)
	}
}

func formatDuration(d time.Duration) string {
	days := int(d.Hours() / 24)
	switch days {
	case 0:
		hours := int(d.Hours())
		if hours == 0 {
			minutes := int(d.Minutes())
			return fmt.Sprintf("%d minutes", minutes)
		}
		return fmt.Sprintf("%d hours", hours)
	case 1:
		return "1 day"
	default:
		return fmt.Sprintf("%d days", days)
	}
}

func DisplayProgress(message string) {
	cyan := color.New(color.FgCyan)
	_, _ = cyan.Printf("⏳ %s...\n", message)
}

func DisplaySuccess(message string) {
	green := color.New(color.FgGreen)
	_, _ = green.Printf("✓ %s\n", message)
}

func DisplayWarning(message string) {
	yellow := color.New(color.FgYellow)
	_, _ = yellow.Printf("⚠ %s\n", message)
}

func DisplayError(message string) {
	red := color.New(color.FgRed, color.Bold)
	_, _ = red.Printf("✗ %s\n", message)
}

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

	cyan.Println("\n" + strings.Repeat("=", 80))
	cyan.Printf("  GitHub Statistics for @%s\n", stats.Username)
	cyan.Println(strings.Repeat("=", 80))

	fmt.Println()
	green.Println("👤 PROFILE")
	fmt.Println(strings.Repeat("-", 80))

	table := tablewriter.NewWriter(os.Stdout)
	table.SetHeader([]string{"Field", "Value"})
	table.SetBorder(false)
	table.SetColumnSeparator("")
	table.SetRowSeparator("")
	table.SetHeaderLine(false)
	table.SetAlignment(tablewriter.ALIGN_LEFT)

	if stats.Name != "" {
		table.Append([]string{"Name", stats.Name})
	}
	table.Append([]string{"Username", stats.Username})
	if stats.Bio != "" {
		table.Append([]string{"Bio", truncate(stats.Bio, 60)})
	}
	if stats.Company != "" {
		table.Append([]string{"Company", stats.Company})
	}
	if stats.Location != "" {
		table.Append([]string{"Location", stats.Location})
	}
	if stats.Blog != "" {
		table.Append([]string{"Website", stats.Blog})
	}
	table.Append([]string{"Joined", stats.CreatedAt.Format("January 2, 2006")})
	table.Append([]string{"Account Age", stats.AccountAge.String()})
	table.Append([]string{"Followers", fmt.Sprintf("%d", stats.Followers)})
	table.Append([]string{"Following", fmt.Sprintf("%d", stats.Following)})

	table.Render()

	fmt.Println()
	green.Println("📚 REPOSITORY STATISTICS")
	fmt.Println(strings.Repeat("-", 80))

	table = tablewriter.NewWriter(os.Stdout)
	table.SetHeader([]string{"Metric", "Value"})
	table.SetBorder(false)
	table.SetColumnSeparator("")
	table.SetRowSeparator("")
	table.SetHeaderLine(false)
	table.SetAlignment(tablewriter.ALIGN_LEFT)

	table.Append([]string{"Public Repositories", fmt.Sprintf("%d", stats.PublicRepos)})
	table.Append([]string{"Public Gists", fmt.Sprintf("%d", stats.PublicGists)})
	table.Append([]string{"Total Stars Received", fmt.Sprintf("%d ⭐", stats.TotalStars)})
	table.Append([]string{"Total Forks Received", fmt.Sprintf("%d", stats.TotalForks)})

	table.Render()

	fmt.Println()
	green.Println("🔥 COMMIT STREAKS")
	fmt.Println(strings.Repeat("-", 80))

	table = tablewriter.NewWriter(os.Stdout)
	table.SetHeader([]string{"Metric", "Value"})
	table.SetBorder(false)
	table.SetColumnSeparator("")
	table.SetRowSeparator("")
	table.SetHeaderLine(false)
	table.SetAlignment(tablewriter.ALIGN_LEFT)

	if stats.CurrentStreak > 0 {
		table.Append([]string{"Current Streak", fmt.Sprintf("%d days 🔥", stats.CurrentStreak)})
		table.Append([]string{"Current Streak Start", stats.CurrentStreakStart.Format("Jan 2, 2006")})
	} else {
		table.Append([]string{"Current Streak", "0 days (inactive)"})
	}

	table.Append([]string{"Maximum Streak", fmt.Sprintf("%d days 🏆", stats.MaxStreak)})
	if !stats.MaxStreakStart.IsZero() {
		streakRange := fmt.Sprintf("%s - %s",
			stats.MaxStreakStart.Format("Jan 2, 2006"),
			stats.MaxStreakEnd.Format("Jan 2, 2006"))
		table.Append([]string{"Max Streak Period", streakRange})
	}
	table.Append([]string{"Total Commit Days", fmt.Sprintf("%d", stats.TotalCommitDays)})

	table.Render()

	if stats.MostActiveDay != "" || stats.MostActiveHour > 0 {
		fmt.Println()
		green.Println("📊 ACTIVITY PATTERNS")
		fmt.Println(strings.Repeat("-", 80))

		table = tablewriter.NewWriter(os.Stdout)
		table.SetHeader([]string{"Metric", "Value"})
		table.SetBorder(false)
		table.SetColumnSeparator("")
		table.SetRowSeparator("")
		table.SetHeaderLine(false)
		table.SetAlignment(tablewriter.ALIGN_LEFT)

		if stats.MostActiveDay != "" {
			table.Append([]string{"Most Active Day", stats.MostActiveDay})
		}
		if stats.MostActiveHour >= 0 {
			hourStr := formatHour(stats.MostActiveHour)
			table.Append([]string{"Most Active Hour", hourStr})
		}

		table.Render()
	}

	if len(stats.Languages) > 0 {
		fmt.Println()
		green.Println("💻 LANGUAGE STATISTICS")
		fmt.Println(strings.Repeat("-", 80))

		langStats := github.GetLanguageStats(stats.Languages)

		table = tablewriter.NewWriter(os.Stdout)
		table.SetHeader([]string{"Language", "Bytes", "Percentage"})
		table.SetBorder(false)
		table.SetColumnSeparator(" | ")
		table.SetHeaderLine(false)
		table.SetAlignment(tablewriter.ALIGN_LEFT)

		count := 10
		if len(langStats.TopLanguages) < count {
			count = len(langStats.TopLanguages)
		}

		for i := 0; i < count; i++ {
			lang := langStats.TopLanguages[i]
			bar := createBar(lang.Percentage, 30)
			table.Append([]string{
				lang.Name,
				formatBytes(lang.Bytes),
				fmt.Sprintf("%.1f%% %s", lang.Percentage, bar),
			})
		}

		table.Render()
	}

	if len(stats.TopRepositories) > 0 {
		fmt.Println()
		green.Println("🌟 TOP REPOSITORIES (by stars)")
		fmt.Println(strings.Repeat("-", 80))

		table = tablewriter.NewWriter(os.Stdout)
		table.SetHeader([]string{"Repository", "Stars", "Forks", "Language"})
		table.SetBorder(false)
		table.SetColumnSeparator(" | ")
		table.SetHeaderLine(false)
		table.SetAlignment(tablewriter.ALIGN_LEFT)

		for _, repo := range stats.TopRepositories {
			lang := repo.Language
			if lang == "" {
				lang = "N/A"
			}
			table.Append([]string{
				repo.Name,
				fmt.Sprintf("%d ⭐", repo.Stars),
				fmt.Sprintf("%d", repo.Forks),
				lang,
			})
		}

		table.Render()
	}

	fmt.Println()
	blue.Println(strings.Repeat("-", 80))
	blue.Printf("Generated at: %s\n", time.Now().Format("2006-01-02 15:04:05 MST"))
	blue.Println(strings.Repeat("=", 80))
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

func createBar(percentage float64, width int) string {
	filled := int(percentage / 100.0 * float64(width))
	if filled > width {
		filled = width
	}
	bar := strings.Repeat("█", filled) + strings.Repeat("░", width-filled)
	return bar
}

func DisplayProgress(message string) {
	cyan := color.New(color.FgCyan)
	cyan.Printf("⏳ %s...\n", message)
}

func DisplaySuccess(message string) {
	green := color.New(color.FgGreen)
	green.Printf("✓ %s\n", message)
}

func DisplayWarning(message string) {
	yellow := color.New(color.FgYellow)
	yellow.Printf("⚠ %s\n", message)
}

func DisplayError(message string) {
	red := color.New(color.FgRed, color.Bold)
	red.Printf("✗ %s\n", message)
}

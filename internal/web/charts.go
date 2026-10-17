package web

import (
	"time"

	"github-stats/internal/github"
)

// chartData is embedded in the page as JSON and drawn by charts.js.
type chartData struct {
	Today    string         `json:"today"`
	Days     []dayPoint     `json:"days"`
	Monthly  []monthPoint   `json:"monthly"`
	Weekdays []weekdayPoint `json:"weekdays"`
	Years    []yearPoint    `json:"years"`
	Repos    []repoPoint    `json:"repos"`
}

type dayPoint struct {
	Date  string `json:"date"`
	Count int    `json:"count"`
}

type monthPoint struct {
	Month string `json:"month"`
	Count int    `json:"count"`
}

type weekdayPoint struct {
	Name    string  `json:"name"`
	Total   int     `json:"total"`
	Average float64 `json:"average"`
}

type yearPoint struct {
	Year         int `json:"year"`
	Total        int `json:"total"`
	Commits      int `json:"commits"`
	PullRequests int `json:"pullRequests"`
	Issues       int `json:"issues"`
	Reviews      int `json:"reviews"`
	Other        int `json:"other"`
}

type repoPoint struct {
	Year    int `json:"year"`
	Sources int `json:"sources"`
	Forks   int `json:"forks"`
}

type contributionSummary struct {
	Total     int
	ThisYear  int
	BestCount int
	BestDate  time.Time
	ActiveAvg float64
}

func buildChartData(stats *github.UserStats, now time.Time) chartData {
	data := chartData{Today: now.Format("2006-01-02")}

	for _, day := range stats.Contributions {
		data.Days = append(data.Days, dayPoint{Date: day.Date.Format("2006-01-02"), Count: day.Count})
	}
	data.Monthly = monthlyContributions(stats.Contributions, now)
	data.Weekdays = weekdayContributions(stats.Contributions, now)

	for _, y := range stats.ContributionYears {
		known := y.Commits + y.PullRequests + y.Issues + y.Reviews
		data.Years = append(data.Years, yearPoint{
			Year:         y.Year,
			Total:        max(y.Total, known),
			Commits:      y.Commits,
			PullRequests: y.PullRequests,
			Issues:       y.Issues,
			Reviews:      y.Reviews,
			Other:        max(0, y.Total-known),
		})
	}

	for _, r := range stats.ReposByYear {
		data.Repos = append(data.Repos, repoPoint{Year: r.Year, Sources: r.Sources, Forks: r.Forks})
	}

	return data
}

func summarizeContributions(days []github.ContributionDay, now time.Time) *contributionSummary {
	if len(days) == 0 {
		return nil
	}

	summary := &contributionSummary{}
	for _, day := range days {
		summary.Total += day.Count
		if day.Date.Year() == now.Year() {
			summary.ThisYear += day.Count
		}
		if day.Count > summary.BestCount {
			summary.BestCount = day.Count
			summary.BestDate = day.Date
		}
	}
	summary.ActiveAvg = float64(summary.Total) / float64(len(days))

	return summary
}

// monthlyContributions totals each month from the first active month to now,
// including months with no contributions.
func monthlyContributions(days []github.ContributionDay, now time.Time) []monthPoint {
	if len(days) == 0 {
		return nil
	}

	totals := make(map[string]int)
	for _, day := range days {
		totals[day.Date.Format("2006-01")] += day.Count
	}

	first := days[0].Date
	month := time.Date(first.Year(), first.Month(), 1, 0, 0, 0, 0, time.UTC)
	last := time.Date(now.Year(), now.Month(), 1, 0, 0, 0, 0, time.UTC)

	var points []monthPoint
	for ; !month.After(last); month = month.AddDate(0, 1, 0) {
		key := month.Format("2006-01")
		points = append(points, monthPoint{Month: key, Count: totals[key]})
	}

	return points
}

// weekdayContributions averages contributions per weekday over every day from
// the first active day to now, so idle days count too.
func weekdayContributions(days []github.ContributionDay, now time.Time) []weekdayPoint {
	if len(days) == 0 {
		return nil
	}

	var totals, occurrences [7]int
	for _, day := range days {
		totals[day.Date.Weekday()] += day.Count
	}

	today := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, time.UTC)
	for d := days[0].Date; !d.After(today); d = d.AddDate(0, 0, 1) {
		occurrences[d.Weekday()]++
	}

	// Monday first, as most calendars outside the US do.
	order := []time.Weekday{time.Monday, time.Tuesday, time.Wednesday, time.Thursday, time.Friday, time.Saturday, time.Sunday}
	points := make([]weekdayPoint, 0, len(order))
	for _, wd := range order {
		point := weekdayPoint{Name: wd.String()[:3], Total: totals[wd]}
		if occurrences[wd] > 0 {
			point.Average = float64(totals[wd]) / float64(occurrences[wd])
		}
		points = append(points, point)
	}

	return points
}

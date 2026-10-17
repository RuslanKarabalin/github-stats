package web

import (
	"strings"
	"testing"
	"time"

	"github-stats/internal/github"
)

func day(s string, count int) github.ContributionDay {
	date, _ := time.Parse("2006-01-02", s)
	return github.ContributionDay{Date: date, Count: count}
}

func TestMonthlyContributionsFillsGaps(t *testing.T) {
	now := time.Date(2026, time.April, 10, 0, 0, 0, 0, time.UTC)
	days := []github.ContributionDay{day("2026-01-05", 3), day("2026-01-20", 2), day("2026-03-01", 4)}

	got := monthlyContributions(days, now)
	want := []monthPoint{{"2026-01", 5}, {"2026-02", 0}, {"2026-03", 4}, {"2026-04", 0}}

	if len(got) != len(want) {
		t.Fatalf("got %d months, want %d: %v", len(got), len(want), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("month %d: got %v, want %v", i, got[i], want[i])
		}
	}
}

func TestWeekdayContributionsAveragesOverIdleDays(t *testing.T) {
	// 2026-01-05 is a Monday; the range covers two Mondays.
	now := time.Date(2026, time.January, 12, 15, 0, 0, 0, time.UTC)
	days := []github.ContributionDay{day("2026-01-05", 4)}

	got := weekdayContributions(days, now)

	if got[0].Name != "Mon" || got[0].Total != 4 || got[0].Average != 2 {
		t.Errorf("Monday: got %+v, want total 4 and average 2", got[0])
	}
	if got[6].Name != "Sun" || got[6].Total != 0 {
		t.Errorf("Sunday: got %+v, want total 0", got[6])
	}
}

func TestSummarizeContributions(t *testing.T) {
	now := time.Date(2026, time.June, 1, 0, 0, 0, 0, time.UTC)
	days := []github.ContributionDay{day("2025-12-31", 6), day("2026-02-01", 9), day("2026-03-01", 3)}

	got := summarizeContributions(days, now)

	if got.Total != 18 || got.ThisYear != 12 || got.BestCount != 9 || got.ActiveAvg != 6 {
		t.Errorf("got %+v", got)
	}
	if got.BestDate.Format("2006-01-02") != "2026-02-01" {
		t.Errorf("best date: got %s", got.BestDate)
	}
	if summarizeContributions(nil, now) != nil {
		t.Error("want nil summary without contributions")
	}
}

func TestBuildChartDataFoldsUnknownTypesIntoOther(t *testing.T) {
	stats := &github.UserStats{
		ContributionYears: []github.ContributionYear{
			{Year: 2026, Total: 20, Commits: 10, PullRequests: 3, Issues: 2, Reviews: 1},
		},
	}

	got := buildChartData(stats, time.Now())

	if got.Years[0].Other != 4 || got.Years[0].Total != 20 {
		t.Errorf("got %+v, want other 4 and total 20", got.Years[0])
	}
}

func TestRenderEmbedsChartData(t *testing.T) {
	stats := &github.UserStats{
		Username:       "octocat",
		MostActiveHour: -1,
		Contributions:  []github.ContributionDay{day("2026-01-05", 3)},
		ContributionYears: []github.ContributionYear{
			{Year: 2026, Total: 3, Commits: 3},
		},
		PRStats: &github.PullRequestStats{Total: 4, Merged: 2, Open: 1, Closed: 1},
	}

	page, err := render(stats)
	if err != nil {
		t.Fatal(err)
	}

	html := string(page)
	for _, want := range []string{`id="chart-data"`, `"date":"2026-01-05"`, `data-chart="heatmap"`, `data-chart="mix"`, "Closed unmerged"} {
		if !strings.Contains(html, want) {
			t.Errorf("rendered page is missing %q", want)
		}
	}
}

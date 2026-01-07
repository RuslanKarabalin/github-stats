package github

import (
	"context"
	"fmt"
	"sort"
	"time"

	"github.com/google/go-github/v57/github"
)

type StatsCalculator struct {
	client *Client
}

func NewStatsCalculator(client *Client) *StatsCalculator {
	return &StatsCalculator{client: client}
}

func (s *StatsCalculator) Calculate(ctx context.Context, username string, fullScan bool) (*UserStats, error) {
	stats := &UserStats{
		Username:  username,
		Languages: make(map[string]int64),
	}

	user, err := s.client.GetUser(username)
	if err != nil {
		return nil, err
	}

	s.populateProfile(stats, user)

	repos, err := s.client.GetRepositories(username)
	if err != nil {
		return nil, fmt.Errorf("failed to get repositories: %w", err)
	}

	s.calculateRepoStats(stats, repos)

	languages, err := s.client.GetLanguages(repos)
	if err != nil {
		fmt.Printf("Warning: failed to get complete language stats: %v\n", err)
	}
	stats.Languages = languages

	commitDates, err := s.client.GetCommitActivity(username, fullScan)
	if err != nil {
		return nil, fmt.Errorf("failed to get commit activity: %w", err)
	}

	streakInfo := s.calculateStreaks(commitDates)
	stats.CurrentStreak = streakInfo.CurrentStreak
	stats.MaxStreak = streakInfo.MaxStreak
	stats.CurrentStreakStart = streakInfo.CurrentStart
	stats.MaxStreakStart = streakInfo.MaxStart
	stats.MaxStreakEnd = streakInfo.MaxEnd
	stats.TotalCommitDays = len(streakInfo.CommitDates)

	s.calculateActivityPatterns(stats, commitDates)
	s.calculateTopRepositories(stats, repos)

	return stats, nil
}

func (s *StatsCalculator) populateProfile(stats *UserStats, user *github.User) {
	if user.Name != nil {
		stats.Name = *user.Name
	}
	if user.Bio != nil {
		stats.Bio = *user.Bio
	}
	if user.Company != nil {
		stats.Company = *user.Company
	}
	if user.Location != nil {
		stats.Location = *user.Location
	}
	if user.Email != nil {
		stats.Email = *user.Email
	}
	if user.Blog != nil {
		stats.Blog = *user.Blog
	}
	if user.CreatedAt != nil {
		stats.CreatedAt = user.CreatedAt.Time
		stats.AccountAge = calculateDuration(user.CreatedAt.Time, time.Now())
	}
	if user.UpdatedAt != nil {
		stats.UpdatedAt = user.UpdatedAt.Time
	}
	if user.PublicRepos != nil {
		stats.PublicRepos = *user.PublicRepos
	}
	if user.PublicGists != nil {
		stats.PublicGists = *user.PublicGists
	}
	if user.Followers != nil {
		stats.Followers = *user.Followers
	}
	if user.Following != nil {
		stats.Following = *user.Following
	}
}

func (s *StatsCalculator) calculateRepoStats(stats *UserStats, repos []*github.Repository) {
	for _, repo := range repos {
		if repo.StargazersCount != nil {
			stats.TotalStars += *repo.StargazersCount
		}
		if repo.ForksCount != nil {
			stats.TotalForks += *repo.ForksCount
		}
	}
}

func (s *StatsCalculator) calculateStreaks(commitDates []time.Time) *StreakInfo {
	if len(commitDates) == 0 {
		return &StreakInfo{}
	}

	dateSet := make(map[string]time.Time)
	for _, date := range commitDates {
		normalized := time.Date(date.Year(), date.Month(), date.Day(), 0, 0, 0, 0, time.UTC)
		dateStr := normalized.Format("2006-01-02")
		if _, exists := dateSet[dateStr]; !exists {
			dateSet[dateStr] = normalized
		}
	}

	var uniqueDates []time.Time
	for _, date := range dateSet {
		uniqueDates = append(uniqueDates, date)
	}
	sort.Slice(uniqueDates, func(i, j int) bool {
		return uniqueDates[i].Before(uniqueDates[j])
	})

	info := &StreakInfo{
		CommitDates: uniqueDates,
	}

	if len(uniqueDates) == 0 {
		return info
	}

	currentStreak := 1
	maxStreak := 1
	currentStart := uniqueDates[0]
	maxStart := uniqueDates[0]
	maxEnd := uniqueDates[0]

	for i := 1; i < len(uniqueDates); i++ {
		daysDiff := int(uniqueDates[i].Sub(uniqueDates[i-1]).Hours() / 24)

		if daysDiff == 1 {
			currentStreak++
		} else if daysDiff == 0 {
			continue
		} else {
			if currentStreak > maxStreak {
				maxStreak = currentStreak
				maxStart = currentStart
				maxEnd = uniqueDates[i-1]
			}
			currentStreak = 1
			currentStart = uniqueDates[i]
		}
	}

	if currentStreak > maxStreak {
		maxStreak = currentStreak
		maxStart = currentStart
		maxEnd = uniqueDates[len(uniqueDates)-1]
	}

	now := time.Now().UTC()
	today := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, time.UTC)
	lastCommit := uniqueDates[len(uniqueDates)-1]
	daysSinceLastCommit := int(today.Sub(lastCommit).Hours() / 24)

	if daysSinceLastCommit <= 1 {
		info.CurrentStreak = currentStreak
		info.CurrentStart = currentStart
	} else {
		info.CurrentStreak = 0
	}

	info.MaxStreak = maxStreak
	info.MaxStart = maxStart
	info.MaxEnd = maxEnd

	return info
}

func (s *StatsCalculator) calculateActivityPatterns(stats *UserStats, commitDates []time.Time) {
	if len(commitDates) == 0 {
		return
	}

	dayCount := make(map[time.Weekday]int)
	hourCount := make(map[int]int)

	for _, date := range commitDates {
		dayCount[date.Weekday()]++
		hourCount[date.Hour()]++
	}

	maxDayCount := 0
	var mostActiveDay time.Weekday
	for day, count := range dayCount {
		if count > maxDayCount {
			maxDayCount = count
			mostActiveDay = day
		}
	}
	stats.MostActiveDay = mostActiveDay.String()

	maxHourCount := 0
	for hour, count := range hourCount {
		if count > maxHourCount {
			maxHourCount = count
			stats.MostActiveHour = hour
		}
	}
}

func (s *StatsCalculator) calculateTopRepositories(stats *UserStats, repos []*github.Repository) {
	var repoList []Repository

	for _, repo := range repos {
		if repo.Fork != nil && *repo.Fork {
			continue
		}

		r := Repository{
			IsForked: false,
		}
		if repo.Name != nil {
			r.Name = *repo.Name
		}
		if repo.Description != nil {
			r.Description = *repo.Description
		}
		if repo.StargazersCount != nil {
			r.Stars = *repo.StargazersCount
		}
		if repo.ForksCount != nil {
			r.Forks = *repo.ForksCount
		}
		if repo.Language != nil {
			r.Language = *repo.Language
		}
		if repo.CreatedAt != nil {
			r.CreatedAt = repo.CreatedAt.Time
		}
		if repo.UpdatedAt != nil {
			r.UpdatedAt = repo.UpdatedAt.Time
		}

		repoList = append(repoList, r)
	}

	sort.Slice(repoList, func(i, j int) bool {
		return repoList[i].Stars > repoList[j].Stars
	})

	if len(repoList) > 5 {
		stats.TopRepositories = repoList[:5]
	} else {
		stats.TopRepositories = repoList
	}
}

func calculateDuration(start, end time.Time) Duration {
	years := 0
	months := 0

	for start.AddDate(years+1, 0, 0).Before(end) || start.AddDate(years+1, 0, 0).Equal(end) {
		years++
	}

	start = start.AddDate(years, 0, 0)
	for start.AddDate(0, months+1, 0).Before(end) || start.AddDate(0, months+1, 0).Equal(end) {
		months++
	}

	start = start.AddDate(0, months, 0)
	days := int(end.Sub(start).Hours() / 24)

	return Duration{
		Years:  years,
		Months: months,
		Days:   days,
	}
}

func GetLanguageStats(languages map[string]int64) *LanguageStats {
	stats := &LanguageStats{
		Languages: languages,
	}

	for _, bytes := range languages {
		stats.TotalBytes += bytes
	}

	for lang, bytes := range languages {
		percentage := 0.0
		if stats.TotalBytes > 0 {
			percentage = float64(bytes) / float64(stats.TotalBytes) * 100.0
		}
		stats.TopLanguages = append(stats.TopLanguages, LanguageStat{
			Name:       lang,
			Bytes:      bytes,
			Percentage: percentage,
		})
	}

	sort.Slice(stats.TopLanguages, func(i, j int) bool {
		return stats.TopLanguages[i].Bytes > stats.TopLanguages[j].Bytes
	})

	return stats
}

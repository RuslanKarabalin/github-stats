package github

import (
	"fmt"
	"time"
)

type UserStats struct {
	Username    string
	Name        string
	Bio         string
	Company     string
	Location    string
	Email       string
	Blog        string
	CreatedAt   time.Time
	UpdatedAt   time.Time
	PublicRepos int
	PublicGists int
	Followers   int
	Following   int

	AccountAge         Duration
	TotalStars         int
	TotalForks         int
	CurrentStreak      int
	MaxStreak          int
	CurrentStreakStart time.Time
	MaxStreakStart     time.Time
	MaxStreakEnd       time.Time
	TotalCommitDays    int

	Languages            map[string]int64
	MostActiveDay        string
	MostActiveHour       int
	TopRepositories      []Repository
	ContributionVelocity float64
	OwnRepoCommits       int
	OtherRepoCommits     int
}

type Repository struct {
	Name        string
	Description string
	Stars       int
	Forks       int
	Language    string
	CreatedAt   time.Time
	UpdatedAt   time.Time
	IsForked    bool
	Commits     int
}

type Duration struct {
	Years  int
	Months int
	Days   int
}

func (d Duration) String() string {
	if d.Years > 0 {
		if d.Months > 0 {
			return fmt.Sprintf("%d years, %d months", d.Years, d.Months)
		}
		return fmt.Sprintf("%d years", d.Years)
	}
	if d.Months > 0 {
		if d.Days > 0 {
			return fmt.Sprintf("%d months, %d days", d.Months, d.Days)
		}
		return fmt.Sprintf("%d months", d.Months)
	}
	return fmt.Sprintf("%d days", d.Days)
}

type StreakInfo struct {
	CurrentStreak int
	MaxStreak     int
	CurrentStart  time.Time
	MaxStart      time.Time
	MaxEnd        time.Time
	CommitDates   []time.Time
}

type LanguageStats struct {
	Languages    map[string]int64
	TotalBytes   int64
	TopLanguages []LanguageStat
}

type LanguageStat struct {
	Name       string
	Bytes      int64
	Percentage float64
}

package github

import (
	"context"
	"fmt"
	"sync"
	"time"

	"github.com/google/go-github/v57/github"
	"golang.org/x/oauth2"
)

type Client struct {
	client     *github.Client
	ctx        context.Context
	maxWorkers int
}

func NewClient(ctx context.Context, token string, maxWorkers int) *Client {
	ts := oauth2.StaticTokenSource(
		&oauth2.Token{AccessToken: token},
	)
	tc := oauth2.NewClient(ctx, ts)

	return &Client{
		client:     github.NewClient(tc),
		ctx:        ctx,
		maxWorkers: maxWorkers,
	}
}

func (c *Client) GetAuthenticatedUser() (string, error) {
	user, _, err := c.client.Users.Get(c.ctx, "")
	if err != nil {
		return "", fmt.Errorf("failed to get authenticated user: %w", err)
	}
	if user.Login == nil {
		return "", fmt.Errorf("authenticated user has no login")
	}
	return *user.Login, nil
}

func (c *Client) GetUser(username string) (*github.User, error) {
	user, resp, err := c.client.Users.Get(c.ctx, username)
	if err != nil {
		if resp != nil && resp.StatusCode == 404 {
			return nil, fmt.Errorf("user '%s' not found", username)
		}
		return nil, fmt.Errorf("failed to get user: %w", err)
	}
	return user, nil
}

func (c *Client) GetRepositories(username string) ([]*github.Repository, error) {
	var allRepos []*github.Repository
	opts := &github.RepositoryListOptions{
		Type:        "owner",
		Sort:        "updated",
		ListOptions: github.ListOptions{PerPage: 100},
	}

	for {
		repos, resp, err := c.client.Repositories.List(c.ctx, username, opts)
		if err != nil {
			return nil, fmt.Errorf("failed to list repositories: %w", err)
		}

		allRepos = append(allRepos, repos...)

		if resp.NextPage == 0 {
			break
		}
		opts.Page = resp.NextPage
	}

	return allRepos, nil
}

func (c *Client) GetLanguages(repos []*github.Repository) (map[string]int64, error) {
	languages := make(map[string]int64)
	var mu sync.Mutex
	var wg sync.WaitGroup

		sem := make(chan struct{}, c.maxWorkers)
	errChan := make(chan error, len(repos))

	for _, repo := range repos {
		if repo.Fork != nil && *repo.Fork {
			continue 		}

		wg.Add(1)
		go func(r *github.Repository) {
			defer wg.Done()
			sem <- struct{}{}        			defer func() { <-sem }() 
			langs, _, err := c.client.Repositories.ListLanguages(c.ctx,
				*r.Owner.Login, *r.Name)
			if err != nil {
				errChan <- fmt.Errorf("failed to get languages for %s: %w", *r.Name, err)
				return
			}

			mu.Lock()
			for lang, bytes := range langs {
				languages[lang] += int64(bytes)
			}
			mu.Unlock()
		}(repo)
	}

	wg.Wait()
	close(errChan)

		var firstErr error
	for err := range errChan {
		if firstErr == nil {
			firstErr = err
		}
	}

	return languages, firstErr
}

func (c *Client) GetCommitActivity(username string, fullScan bool) ([]time.Time, error) {
	if fullScan {
		return c.getCommitActivityFull(username)
	}
	return c.getCommitActivityRecent(username)
}

func (c *Client) getCommitActivityRecent(username string) ([]time.Time, error) {
	var commitDates []time.Time
	dateSet := make(map[string]bool)

	opts := &github.ListOptions{PerPage: 100}
	for page := 1; page <= 10; page++ { 		opts.Page = page
		events, resp, err := c.client.Activity.ListEventsPerformedByUser(c.ctx, username, false, opts)
		if err != nil {
			return nil, fmt.Errorf("failed to list events: %w", err)
		}

		for _, event := range events {
			if event.Type != nil && *event.Type == "PushEvent" {
				if event.CreatedAt != nil {
					dateStr := event.CreatedAt.UTC().Format("2006-01-02")
					if !dateSet[dateStr] {
						dateSet[dateStr] = true
						commitDates = append(commitDates, event.CreatedAt.UTC())
					}
				}
			}
		}

		if resp.NextPage == 0 {
			break
		}
	}

	return commitDates, nil
}

func (c *Client) getCommitActivityFull(username string) ([]time.Time, error) {
	repos, err := c.GetRepositories(username)
	if err != nil {
		return nil, err
	}

	var mu sync.Mutex
	var wg sync.WaitGroup
	dateSet := make(map[string]bool)
	var commitDates []time.Time

	sem := make(chan struct{}, c.maxWorkers)
	errChan := make(chan error, len(repos))

	for _, repo := range repos {
		wg.Add(1)
		go func(r *github.Repository) {
			defer wg.Done()
			sem <- struct{}{}
			defer func() { <-sem }()

			dates, err := c.getRepoCommits(username, *r.Owner.Login, *r.Name)
			if err != nil {
				errChan <- err
				return
			}

			mu.Lock()
			for _, date := range dates {
				dateStr := date.Format("2006-01-02")
				if !dateSet[dateStr] {
					dateSet[dateStr] = true
					commitDates = append(commitDates, date)
				}
			}
			mu.Unlock()
		}(repo)
	}

	wg.Wait()
	close(errChan)

	var firstErr error
	for err := range errChan {
		if firstErr == nil {
			firstErr = err
		}
	}

	return commitDates, firstErr
}

func (c *Client) getRepoCommits(author, owner, repo string) ([]time.Time, error) {
	var dates []time.Time
	opts := &github.CommitsListOptions{
		Author: author,
		ListOptions: github.ListOptions{PerPage: 100},
	}

	for {
		commits, resp, err := c.client.Repositories.ListCommits(c.ctx, owner, repo, opts)
		if err != nil {
						return dates, nil
		}

		for _, commit := range commits {
			if commit.Commit != nil && commit.Commit.Author != nil && commit.Commit.Author.Date != nil {
				dates = append(dates, commit.Commit.Author.Date.UTC())
			}
		}

		if resp.NextPage == 0 {
			break
		}
		opts.Page = resp.NextPage
	}

	return dates, nil
}

func (c *Client) CheckRateLimit() (*github.RateLimits, error) {
	limits, _, err := c.client.RateLimit.Get(c.ctx)
	if err != nil {
		return nil, fmt.Errorf("failed to check rate limit: %w", err)
	}
	return limits, nil
}

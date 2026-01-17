package github

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/google/go-github/v81/github"
	"golang.org/x/oauth2"
)

type Client struct {
	client     *github.Client
	httpClient *http.Client
	token      string
	ctx        context.Context
	maxWorkers int
}

const graphQLEndpoint = "https://api.github.com/graphql"

type graphQLRequest struct {
	Query     string                 `json:"query"`
	Variables map[string]interface{} `json:"variables"`
}

type contributionCalendarResponse struct {
	Data struct {
		User struct {
			ContributionsCollection struct {
				ContributionCalendar struct {
					TotalContributions int `json:"totalContributions"`
					Weeks              []struct {
						ContributionDays []struct {
							Date              string `json:"date"`
							ContributionCount int    `json:"contributionCount"`
						} `json:"contributionDays"`
					} `json:"weeks"`
				} `json:"contributionCalendar"`
			} `json:"contributionsCollection"`
		} `json:"user"`
	} `json:"data"`
	Errors []struct {
		Message string `json:"message"`
	} `json:"errors"`
}

func NewClient(ctx context.Context, token string, maxWorkers int) *Client {
	ts := oauth2.StaticTokenSource(
		&oauth2.Token{AccessToken: token},
	)
	tc := oauth2.NewClient(ctx, ts)

	return &Client{
		client:     github.NewClient(tc),
		httpClient: tc,
		token:      token,
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
	opts := &github.RepositoryListByUserOptions{
		Type:        "owner",
		Sort:        "updated",
		ListOptions: github.ListOptions{PerPage: 100},
	}

	for {
		repos, resp, err := c.client.Repositories.ListByUser(c.ctx, username, opts)
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
			continue
		}

		wg.Add(1)
		go func(r *github.Repository) {
			defer wg.Done()
			sem <- struct{}{}
			defer func() { <-sem }()
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
	dates, err := c.GetContributionCalendar(username)
	if err == nil && len(dates) > 0 {
		return dates, nil
	}

	if fullScan {
		return c.getCommitActivityFull(username)
	}
	return c.getCommitActivityRecent(username)
}

func (c *Client) getCommitActivityRecent(username string) ([]time.Time, error) {
	var commitDates []time.Time
	dateSet := make(map[string]bool)

	opts := &github.ListOptions{PerPage: 100}
	for page := 1; page <= 10; page++ {
		opts.Page = page
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
		Author:      author,
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

func (c *Client) GetContributionCalendar(username string) ([]time.Time, error) {
	now := time.Now().UTC()
	var allDates []time.Time
	dateSet := make(map[string]bool)

	for yearsBack := 0; yearsBack < 5; yearsBack++ {
		to := now.AddDate(-yearsBack, 0, 0)
		from := to.AddDate(-1, 0, 0)

		if from.After(now) {
			continue
		}
		if to.After(now) {
			to = now
		}

		dates, err := c.getContributionsForPeriod(username, from, to)
		if err != nil {
			if yearsBack > 0 {
				break
			}
			return nil, err
		}

		for _, date := range dates {
			dateStr := date.Format("2006-01-02")
			if !dateSet[dateStr] {
				dateSet[dateStr] = true
				allDates = append(allDates, date)
			}
		}
	}

	return allDates, nil
}

func (c *Client) getContributionsForPeriod(username string, from, to time.Time) ([]time.Time, error) {
	query := `
		query($username: String!, $from: DateTime!, $to: DateTime!) {
			user(login: $username) {
				contributionsCollection(from: $from, to: $to) {
					contributionCalendar {
						totalContributions
						weeks {
							contributionDays {
								date
								contributionCount
							}
						}
					}
				}
			}
		}
	`

	variables := map[string]interface{}{
		"username": username,
		"from":     from.Format(time.RFC3339),
		"to":       to.Format(time.RFC3339),
	}

	reqBody := graphQLRequest{
		Query:     query,
		Variables: variables,
	}

	jsonBody, err := json.Marshal(reqBody)
	if err != nil {
		return nil, fmt.Errorf("failed to marshal request: %w", err)
	}

	req, err := http.NewRequestWithContext(c.ctx, "POST", graphQLEndpoint, bytes.NewBuffer(jsonBody))
	if err != nil {
		return nil, fmt.Errorf("failed to create request: %w", err)
	}

	req.Header.Set("Content-Type", "application/json")

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return nil, fmt.Errorf("failed to execute request: %w", err)
	}
	defer func() { _ = resp.Body.Close() }()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("failed to read response: %w", err)
	}

	var result contributionCalendarResponse
	if err := json.Unmarshal(body, &result); err != nil {
		return nil, fmt.Errorf("failed to parse response: %w", err)
	}

	if len(result.Errors) > 0 {
		return nil, fmt.Errorf("GraphQL error: %s", result.Errors[0].Message)
	}

	var dates []time.Time
	for _, week := range result.Data.User.ContributionsCollection.ContributionCalendar.Weeks {
		for _, day := range week.ContributionDays {
			if day.ContributionCount > 0 {
				date, err := time.Parse("2006-01-02", day.Date)
				if err != nil {
					continue
				}
				dates = append(dates, date)
			}
		}
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

func (c *Client) GetUserPullRequests(username string) (*PullRequestStats, error) {
	stats := &PullRequestStats{
		TopRepos: make([]RepoCount, 0),
	}

	repoCount := make(map[string]int)
	var mergeTimes []time.Duration

	query := fmt.Sprintf("author:%s is:pr", username)
	opts := &github.SearchOptions{
		Sort:        "created",
		ListOptions: github.ListOptions{PerPage: 100},
	}

	for {
		result, resp, err := c.client.Search.Issues(c.ctx, query, opts)
		if err != nil {
			return nil, fmt.Errorf("failed to search PRs: %w", err)
		}

		for _, issue := range result.Issues {
			stats.Total++

			if issue.RepositoryURL != nil {
				repoName := extractRepoName(*issue.RepositoryURL)
				repoCount[repoName]++
			}

			if issue.State != nil {
				switch *issue.State {
				case "open":
					stats.Open++
				case "closed":
					stats.Closed++
				}
			}
		}

		if resp.NextPage == 0 {
			break
		}
		opts.Page = resp.NextPage
	}

	mergedQuery := fmt.Sprintf("author:%s is:pr is:merged", username)
	opts.Page = 0

	for {
		result, resp, err := c.client.Search.Issues(c.ctx, mergedQuery, opts)
		if err != nil {
			return nil, fmt.Errorf("failed to search merged PRs: %w", err)
		}

		for _, issue := range result.Issues {
			stats.Merged++
			if issue.CreatedAt != nil && issue.ClosedAt != nil {
				mergeTime := issue.ClosedAt.Sub(issue.CreatedAt.Time)
				mergeTimes = append(mergeTimes, mergeTime)
			}
		}

		if resp.NextPage == 0 {
			break
		}
		opts.Page = resp.NextPage
	}

	stats.Closed = stats.Closed - stats.Merged

	if len(mergeTimes) > 0 {
		var total time.Duration
		for _, t := range mergeTimes {
			total += t
		}
		stats.AvgMergeTime = total / time.Duration(len(mergeTimes))
	}

	stats.TopRepos = getTopRepos(repoCount, 5)

	return stats, nil
}

func (c *Client) GetUserIssues(username string) (*IssueStats, error) {
	stats := &IssueStats{}
	var closeTimes []time.Duration

	query := fmt.Sprintf("author:%s is:issue", username)
	opts := &github.SearchOptions{
		Sort:        "created",
		ListOptions: github.ListOptions{PerPage: 100},
	}

	for {
		result, resp, err := c.client.Search.Issues(c.ctx, query, opts)
		if err != nil {
			return nil, fmt.Errorf("failed to search issues: %w", err)
		}

		for _, issue := range result.Issues {
			stats.Total++

			if issue.State != nil {
				switch *issue.State {
				case "open":
					stats.Open++
				case "closed":
					stats.Closed++
					if issue.CreatedAt != nil && issue.ClosedAt != nil {
						closeTime := issue.ClosedAt.Sub(issue.CreatedAt.Time)
						closeTimes = append(closeTimes, closeTime)
					}
				}
			}
		}

		if resp.NextPage == 0 {
			break
		}
		opts.Page = resp.NextPage
	}

	if len(closeTimes) > 0 {
		var total time.Duration
		for _, t := range closeTimes {
			total += t
		}
		stats.AvgCloseTime = total / time.Duration(len(closeTimes))
	}

	return stats, nil
}

type reviewContributionsResponse struct {
	Data struct {
		User struct {
			ContributionsCollection struct {
				PullRequestReviewContributions struct {
					TotalCount int `json:"totalCount"`
					Nodes      []struct {
						PullRequest struct {
							Repository struct {
								NameWithOwner string `json:"nameWithOwner"`
							} `json:"repository"`
						} `json:"pullRequest"`
					} `json:"nodes"`
					PageInfo struct {
						HasNextPage bool   `json:"hasNextPage"`
						EndCursor   string `json:"endCursor"`
					} `json:"pageInfo"`
				} `json:"pullRequestReviewContributions"`
			} `json:"contributionsCollection"`
		} `json:"user"`
	} `json:"data"`
	Errors []struct {
		Message string `json:"message"`
	} `json:"errors"`
}

func (c *Client) GetUserReviews(username string) (*ReviewStats, error) {
	stats := &ReviewStats{
		TopRepos: make([]RepoCount, 0),
	}

	repoCount := make(map[string]int)
	var cursor *string

	for {
		query := `
			query($username: String!, $after: String) {
				user(login: $username) {
					contributionsCollection {
						pullRequestReviewContributions(first: 100, after: $after) {
							totalCount
							nodes {
								pullRequest {
									repository {
										nameWithOwner
									}
								}
							}
							pageInfo {
								hasNextPage
								endCursor
							}
						}
					}
				}
			}
		`

		variables := map[string]interface{}{
			"username": username,
		}
		if cursor != nil {
			variables["after"] = *cursor
		}

		reqBody := graphQLRequest{
			Query:     query,
			Variables: variables,
		}

		jsonBody, err := json.Marshal(reqBody)
		if err != nil {
			return nil, fmt.Errorf("failed to marshal request: %w", err)
		}

		req, err := http.NewRequestWithContext(c.ctx, "POST", graphQLEndpoint, bytes.NewBuffer(jsonBody))
		if err != nil {
			return nil, fmt.Errorf("failed to create request: %w", err)
		}

		req.Header.Set("Content-Type", "application/json")

		resp, err := c.httpClient.Do(req)
		if err != nil {
			return nil, fmt.Errorf("failed to execute request: %w", err)
		}

		body, err := io.ReadAll(resp.Body)
		_ = resp.Body.Close()
		if err != nil {
			return nil, fmt.Errorf("failed to read response: %w", err)
		}

		var result reviewContributionsResponse
		if err := json.Unmarshal(body, &result); err != nil {
			return nil, fmt.Errorf("failed to parse response: %w", err)
		}

		if len(result.Errors) > 0 {
			return nil, fmt.Errorf("GraphQL error: %s", result.Errors[0].Message)
		}

		contributions := result.Data.User.ContributionsCollection.PullRequestReviewContributions

		if stats.Total == 0 {
			stats.Total = contributions.TotalCount
		}

		for _, node := range contributions.Nodes {
			repoName := node.PullRequest.Repository.NameWithOwner
			repoCount[repoName]++
		}

		if !contributions.PageInfo.HasNextPage {
			break
		}
		cursor = &contributions.PageInfo.EndCursor
	}

	stats.TopRepos = getTopRepos(repoCount, 5)

	return stats, nil
}

func extractRepoName(repoURL string) string {
	parts := strings.Split(repoURL, "/repos/")
	if len(parts) == 2 {
		return parts[1]
	}
	return repoURL
}

func getTopRepos(repoCount map[string]int, limit int) []RepoCount {
	var repos []RepoCount
	for name, count := range repoCount {
		repos = append(repos, RepoCount{RepoName: name, Count: count})
	}

	for i := 0; i < len(repos); i++ {
		for j := i + 1; j < len(repos); j++ {
			if repos[j].Count > repos[i].Count {
				repos[i], repos[j] = repos[j], repos[i]
			}
		}
	}

	if len(repos) > limit {
		return repos[:limit]
	}
	return repos
}

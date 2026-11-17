use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Datelike, Local, NaiveDate, SecondsFormat, TimeDelta, TimeZone, Utc};
use futures::{StreamExt, stream};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, LINK, USER_AGENT};
use reqwest::{StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::types::{
    ContributionCalendar, ContributionDay, ContributionYear, IssueStats, OpenPullRequest,
    PullRequestStats, RepoCount, ReviewStats,
};

const API_URL: &str = "https://api.github.com";
const GRAPHQL_URL: &str = "https://api.github.com/graphql";
const PER_PAGE: u32 = 100;

/// A thin GitHub REST and GraphQL client. Requests that fan out over many
/// repositories run at most `max_workers` at a time.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    max_workers: usize,
}

/// A non-success HTTP response from the REST API.
#[derive(Debug)]
pub struct ApiError {
    pub url: String,
    pub status: StatusCode,
    pub message: String,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GET {}: {}", self.url, self.status)?;
        if !self.message.is_empty() {
            write!(f, " {}", self.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {}

/// One page of a paginated REST response.
struct Page<T> {
    items: T,
    next: Option<u32>,
    last: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ApiUser {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    pub email: Option<String>,
    pub blog: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub public_repos: u64,
    #[serde(default)]
    pub public_gists: u64,
    #[serde(default)]
    pub followers: u64,
    #[serde(default)]
    pub following: u64,
}

#[derive(Debug, Deserialize)]
pub struct ApiRepo {
    pub name: String,
    pub owner: ApiOwner,
    #[serde(default)]
    pub fork: bool,
    pub description: Option<String>,
    #[serde(default)]
    pub stargazers_count: u64,
    #[serde(default)]
    pub forks_count: u64,
    pub language: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ApiOwner {
    pub login: String,
}

#[derive(Debug, Deserialize)]
struct ApiEvent {
    #[serde(rename = "type")]
    kind: Option<String>,
    created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct ApiCommit {
    commit: Option<ApiCommitDetail>,
}

#[derive(Debug, Deserialize)]
struct ApiCommitDetail {
    author: Option<ApiCommitAuthor>,
}

#[derive(Debug, Deserialize)]
struct ApiCommitAuthor {
    date: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    #[serde(default)]
    items: Vec<ApiIssue>,
}

#[derive(Debug, Deserialize)]
struct ApiIssue {
    repository_url: Option<String>,
    state: Option<String>,
    title: Option<String>,
    number: Option<u64>,
    created_at: Option<DateTime<Utc>>,
    closed_at: Option<DateTime<Utc>>,
    html_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RateLimitResponse {
    resources: RateLimitResources,
}

#[derive(Debug, Deserialize)]
struct RateLimitResources {
    core: Option<ApiRate>,
}

#[derive(Debug, Deserialize)]
struct ApiRate {
    limit: u64,
    remaining: u64,
    reset: i64,
}

#[derive(Debug, Clone)]
pub struct Rate {
    pub limit: u64,
    pub remaining: u64,
    pub reset: DateTime<Local>,
}

#[derive(Serialize)]
struct GraphQlRequest<'a> {
    query: &'a str,
    variables: serde_json::Value,
}

#[derive(Deserialize)]
struct GraphQlResponse<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

#[derive(Deserialize)]
struct GraphQlError {
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserData<T> {
    user: Option<UserContributions<T>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserContributions<T> {
    contributions_collection: T,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarCollection {
    total_commit_contributions: u32,
    total_issue_contributions: u32,
    total_pull_request_contributions: u32,
    total_pull_request_review_contributions: u32,
    total_repository_contributions: u32,
    restricted_contributions_count: u32,
    contribution_calendar: CalendarData,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarData {
    total_contributions: u32,
    weeks: Vec<CalendarWeek>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarWeek {
    contribution_days: Vec<CalendarDay>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarDay {
    date: NaiveDate,
    contribution_count: u32,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewCollection {
    pull_request_review_contributions: ReviewContributions,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewContributions {
    total_count: u64,
    nodes: Vec<ReviewNode>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewNode {
    pull_request: ReviewPullRequest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewPullRequest {
    repository: ReviewRepository,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewRepository {
    name_with_owner: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

const CONTRIBUTIONS_QUERY: &str = r"
query($username: String!, $from: DateTime!, $to: DateTime!) {
  user(login: $username) {
    contributionsCollection(from: $from, to: $to) {
      totalCommitContributions
      totalIssueContributions
      totalPullRequestContributions
      totalPullRequestReviewContributions
      totalRepositoryContributions
      restrictedContributionsCount
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
}";

const REVIEWS_QUERY: &str = r"
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
}";

impl Client {
    pub fn new(token: &str, max_workers: usize) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .context("token contains invalid characters")?;
        auth.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth);
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(concat!("github-stats/", env!("CARGO_PKG_VERSION"))),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_static("2022-11-28"),
        );

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            http,
            max_workers: max_workers.max(1),
        })
    }

    async fn get_page<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Page<T>> {
        let url = format!("{API_URL}{path}");
        let resp = self.http.get(&url).query(query).send().await?;
        let status = resp.status();
        let link = resp
            .headers()
            .get(LINK)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let full_url = resp.url().to_string();

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("message")?.as_str().map(str::to_owned))
                .unwrap_or_default();
            return Err(ApiError {
                url: full_url,
                status,
                message,
            }
            .into());
        }

        let items = resp
            .json()
            .await
            .with_context(|| format!("failed to decode response from {full_url}"))?;
        let link = link.as_deref().unwrap_or_default();

        Ok(Page {
            items,
            next: link_page(link, "next"),
            last: link_page(link, "last"),
        })
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Ok(self.get_page(path, &[]).await?.items)
    }

    /// Fetches every page of a REST list endpoint.
    async fn get_all<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Vec<T>> {
        let mut all = Vec::new();
        let mut page = 1;
        loop {
            let mut q = query.to_vec();
            q.push(("per_page", PER_PAGE.to_string()));
            q.push(("page", page.to_string()));

            let resp: Page<Vec<T>> = self.get_page(path, &q).await?;
            all.extend(resp.items);

            match resp.next {
                Some(next) => page = next,
                None => return Ok(all),
            }
        }
    }

    /// Posts a GraphQL query and returns its data, failing on GraphQL errors.
    async fn graphql<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<Option<T>> {
        let resp = self
            .http
            .post(GRAPHQL_URL)
            .json(&GraphQlRequest { query, variables })
            .send()
            .await
            .context("failed to execute request")?;

        if resp.status() != StatusCode::OK {
            bail!("GraphQL request failed: {}", resp.status());
        }

        let body: GraphQlResponse<T> = resp.json().await.context("failed to parse response")?;
        if let Some(err) = body.errors.first() {
            bail!("GraphQL error: {}", err.message);
        }

        Ok(body.data)
    }

    pub async fn authenticated_user(&self) -> Result<String> {
        let user: ApiUser = self
            .get("/user")
            .await
            .context("failed to get authenticated user")?;
        Ok(user.login)
    }

    pub async fn user(&self, username: &str) -> Result<ApiUser> {
        match self.get(&format!("/users/{username}")).await {
            Ok(user) => Ok(user),
            Err(err)
                if err
                    .downcast_ref::<ApiError>()
                    .is_some_and(|e| e.status == StatusCode::NOT_FOUND) =>
            {
                Err(anyhow!("user '{username}' not found"))
            }
            Err(err) => Err(err.context("failed to get user")),
        }
    }

    pub async fn repositories(&self, username: &str) -> Result<Vec<ApiRepo>> {
        let query = [
            ("type", "owner".to_string()),
            ("sort", "updated".to_string()),
        ];
        self.get_all(&format!("/users/{username}/repos"), &query)
            .await
            .context("failed to list repositories")
    }

    /// Sums language bytes over all non-fork repositories. On failure the
    /// totals gathered so far are returned together with the first error.
    pub async fn languages(
        &self,
        repos: &[ApiRepo],
    ) -> (BTreeMap<String, u64>, Option<anyhow::Error>) {
        let results: Vec<_> = stream::iter(repos.iter().filter(|r| !r.fork))
            .map(|repo| async move {
                self.get::<HashMap<String, u64>>(&format!(
                    "/repos/{}/{}/languages",
                    repo.owner.login, repo.name
                ))
                .await
                .with_context(|| format!("failed to get languages for {}", repo.name))
            })
            .buffer_unordered(self.max_workers)
            .collect()
            .await;

        let mut languages = BTreeMap::new();
        let mut first_err = None;
        for result in results {
            match result {
                Ok(langs) => {
                    for (lang, bytes) in langs {
                        *languages.entry(lang).or_default() += bytes;
                    }
                }
                Err(err) => {
                    first_err.get_or_insert(err);
                }
            }
        }

        (languages, first_err)
    }

    /// Collects commit timestamps from the REST API. This is the fallback for
    /// when the contribution calendar is unavailable.
    pub async fn commit_activity(
        &self,
        username: &str,
        full_scan: bool,
    ) -> Result<Vec<DateTime<Utc>>> {
        if full_scan {
            self.commit_activity_full(username).await
        } else {
            self.commit_activity_recent(username).await
        }
    }

    async fn commit_activity_recent(&self, username: &str) -> Result<Vec<DateTime<Utc>>> {
        let mut seen = BTreeSet::new();
        let mut dates = Vec::new();

        for page in 1..=10 {
            let query = [
                ("per_page", PER_PAGE.to_string()),
                ("page", page.to_string()),
            ];
            let resp: Page<Vec<ApiEvent>> = self
                .get_page(&format!("/users/{username}/events"), &query)
                .await
                .context("failed to list events")?;

            for event in resp.items {
                if event.kind.as_deref() != Some("PushEvent") {
                    continue;
                }
                if let Some(at) = event.created_at
                    && seen.insert(at.date_naive())
                {
                    dates.push(at);
                }
            }

            if resp.next.is_none() {
                break;
            }
        }

        Ok(dates)
    }

    async fn commit_activity_full(&self, username: &str) -> Result<Vec<DateTime<Utc>>> {
        let repos = self.repositories(username).await?;

        let per_repo: Vec<Vec<DateTime<Utc>>> = stream::iter(&repos)
            .map(|repo| self.repo_commit_dates(username, &repo.owner.login, &repo.name))
            .buffer_unordered(self.max_workers)
            .collect()
            .await;

        let mut seen = BTreeSet::new();
        Ok(per_repo
            .into_iter()
            .flatten()
            .filter(|at| seen.insert(at.date_naive()))
            .collect())
    }

    /// Commit dates by `author` in one repository. Errors (for example an
    /// empty repository) end the scan with whatever was collected.
    async fn repo_commit_dates(&self, author: &str, owner: &str, repo: &str) -> Vec<DateTime<Utc>> {
        let path = format!("/repos/{owner}/{repo}/commits");
        let mut dates = Vec::new();
        let mut page = 1;

        loop {
            let query = [
                ("author", author.to_string()),
                ("per_page", PER_PAGE.to_string()),
                ("page", page.to_string()),
            ];
            let Ok(resp) = self.get_page::<Vec<ApiCommit>>(&path, &query).await else {
                return dates;
            };

            dates.extend(
                resp.items
                    .into_iter()
                    .filter_map(|c| c.commit?.author?.date),
            );

            match resp.next {
                Some(next) => page = next,
                None => return dates,
            }
        }
    }

    pub async fn top_repos_by_commits(
        &self,
        username: &str,
        repos: &[ApiRepo],
        limit: usize,
    ) -> Vec<RepoCount> {
        let counts: HashMap<String, u64> = stream::iter(repos.iter().filter(|r| !r.fork))
            .map(|repo| async move {
                let count = self
                    .count_repo_commits(username, &repo.owner.login, &repo.name)
                    .await;
                (repo.name.clone(), count)
            })
            .buffer_unordered(self.max_workers)
            .filter(|(_, count)| std::future::ready(*count > 0))
            .collect()
            .await;

        top_repos(counts, limit)
    }

    /// Counts commits by asking for one commit per page and reading the
    /// number of the last page.
    async fn count_repo_commits(&self, author: &str, owner: &str, repo: &str) -> u64 {
        let query = [
            ("author", author.to_string()),
            ("per_page", "1".to_string()),
        ];
        match self
            .get_page::<Vec<serde_json::Value>>(&format!("/repos/{owner}/{repo}/commits"), &query)
            .await
        {
            Ok(page) => page.last.map_or(page.items.len() as u64, u64::from),
            Err(_) => 0,
        }
    }

    /// Fetches the contribution calendar one calendar year at a time, from the
    /// year of `since` up to the current year. If an older year fails, the
    /// years after it are still returned.
    pub async fn contribution_calendar(
        &self,
        username: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<ContributionCalendar> {
        let now = Utc::now();
        let first_year = since.map_or(now.year(), |s| s.year()).min(now.year());

        // Newest year first, so a failure in an old year keeps the recent ones.
        let results: Vec<Result<(Vec<ContributionDay>, ContributionYear)>> =
            stream::iter((first_year..=now.year()).rev())
                .map(|year| async move {
                    let from = Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).unwrap();
                    let to = Utc
                        .with_ymd_and_hms(year, 12, 31, 23, 59, 59)
                        .unwrap()
                        .min(now);
                    let (days, mut summary) =
                        self.contributions_for_period(username, from, to).await?;
                    summary.year = year;
                    Ok((days, summary))
                })
                .buffered(self.max_workers)
                .collect()
                .await;

        let mut results = results.into_iter();
        let mut collected = vec![results.next().expect("at least one year")?];
        collected.extend(results.map_while(Result::ok));
        collected.reverse();

        let mut calendar = ContributionCalendar::default();
        for (days, year) in collected {
            calendar.days.extend(days);
            calendar.years.push(year);
        }

        Ok(calendar)
    }

    async fn contributions_for_period(
        &self,
        username: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<(Vec<ContributionDay>, ContributionYear)> {
        let variables = json!({
            "username": username,
            "from": from.to_rfc3339_opts(SecondsFormat::Secs, true),
            "to": to.to_rfc3339_opts(SecondsFormat::Secs, true),
        });

        let data: Option<UserData<CalendarCollection>> =
            self.graphql(CONTRIBUTIONS_QUERY, variables).await?;
        let c = data
            .and_then(|d| d.user)
            .map(|u| u.contributions_collection)
            .unwrap_or_default();

        let summary = ContributionYear {
            year: 0,
            total: c.contribution_calendar.total_contributions,
            commits: c.total_commit_contributions,
            pull_requests: c.total_pull_request_contributions,
            issues: c.total_issue_contributions,
            reviews: c.total_pull_request_review_contributions,
            repositories: c.total_repository_contributions,
            private: c.restricted_contributions_count,
        };

        let days = c
            .contribution_calendar
            .weeks
            .into_iter()
            .flat_map(|w| w.contribution_days)
            .filter(|d| d.contribution_count > 0)
            .map(|d| ContributionDay {
                date: d.date,
                count: d.contribution_count,
            })
            .collect();

        Ok((days, summary))
    }

    pub async fn rate_limit(&self) -> Result<Option<Rate>> {
        let resp: RateLimitResponse = self
            .get("/rate_limit")
            .await
            .context("failed to check rate limit")?;

        Ok(resp.resources.core.map(|core| Rate {
            limit: core.limit,
            remaining: core.remaining,
            reset: Local
                .timestamp_opt(core.reset, 0)
                .single()
                .unwrap_or_else(Local::now),
        }))
    }

    /// Runs an issue search and returns every result page.
    async fn search_issues(&self, query: &str) -> Result<Vec<ApiIssue>> {
        let mut all = Vec::new();
        let mut page = 1;
        loop {
            let q = [
                ("q", query.to_string()),
                ("sort", "created".to_string()),
                ("per_page", PER_PAGE.to_string()),
                ("page", page.to_string()),
            ];
            let resp: Page<SearchResult> = self.get_page("/search/issues", &q).await?;
            all.extend(resp.items.items);

            match resp.next {
                Some(next) => page = next,
                None => return Ok(all),
            }
        }
    }

    pub async fn pull_requests(&self, username: &str) -> Result<PullRequestStats> {
        let mut stats = PullRequestStats::default();
        let mut repo_count: HashMap<String, u64> = HashMap::new();

        let prs = self
            .search_issues(&format!("author:{username} is:pr"))
            .await
            .context("failed to search PRs")?;

        for pr in prs {
            stats.total += 1;

            let repo_name = pr
                .repository_url
                .as_deref()
                .map(extract_repo_name)
                .unwrap_or_default()
                .to_string();
            if pr.repository_url.is_some() {
                *repo_count.entry(repo_name.clone()).or_default() += 1;
            }

            match pr.state.as_deref() {
                Some("open") => {
                    stats.open += 1;
                    stats.open_prs.push(OpenPullRequest {
                        title: pr.title.unwrap_or_default(),
                        repo_name,
                        number: pr.number.unwrap_or_default(),
                        created_at: pr.created_at,
                        url: pr.html_url.unwrap_or_default(),
                    });
                }
                Some("closed") => stats.closed += 1,
                _ => {}
            }
        }

        let merged = self
            .search_issues(&format!("author:{username} is:pr is:merged"))
            .await
            .context("failed to search merged PRs")?;

        stats.merged = merged.len() as u64;
        stats.closed = stats.closed.saturating_sub(stats.merged);
        stats.avg_merge_time = average(
            merged
                .iter()
                .filter_map(|pr| Some(pr.closed_at? - pr.created_at?)),
        );
        stats.top_repos = top_repos(repo_count, 5);

        Ok(stats)
    }

    pub async fn issues(&self, username: &str) -> Result<IssueStats> {
        let issues = self
            .search_issues(&format!("author:{username} is:issue"))
            .await
            .context("failed to search issues")?;

        let mut stats = IssueStats::default();
        let mut close_times = Vec::new();

        for issue in issues {
            stats.total += 1;
            match issue.state.as_deref() {
                Some("open") => stats.open += 1,
                Some("closed") => {
                    stats.closed += 1;
                    if let (Some(created), Some(closed)) = (issue.created_at, issue.closed_at) {
                        close_times.push(closed - created);
                    }
                }
                _ => {}
            }
        }

        stats.avg_close_time = average(close_times.into_iter());
        Ok(stats)
    }

    pub async fn reviews(&self, username: &str) -> Result<ReviewStats> {
        let mut stats = ReviewStats::default();
        let mut repo_count: HashMap<String, u64> = HashMap::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut variables = json!({ "username": username });
            if let Some(after) = &cursor {
                variables["after"] = json!(after);
            }

            let data: Option<UserData<ReviewCollection>> =
                self.graphql(REVIEWS_QUERY, variables).await?;
            let contributions = data
                .and_then(|d| d.user)
                .map(|u| u.contributions_collection.pull_request_review_contributions)
                .unwrap_or_default();

            if stats.total == 0 {
                stats.total = contributions.total_count;
            }
            for node in contributions.nodes {
                *repo_count
                    .entry(node.pull_request.repository.name_with_owner)
                    .or_default() += 1;
            }

            match contributions.page_info {
                PageInfo {
                    has_next_page: true,
                    end_cursor: Some(next),
                } => cursor = Some(next),
                _ => break,
            }
        }

        stats.top_repos = top_repos(repo_count, 5);
        Ok(stats)
    }
}

/// Reads the `page` number of the link with relation `rel` from a GitHub
/// `Link` header.
fn link_page(header: &str, rel: &str) -> Option<u32> {
    let wanted = format!("rel=\"{rel}\"");
    header.split(',').find_map(|part| {
        let mut pieces = part.split(';').map(str::trim);
        let url = pieces.next()?.strip_prefix('<')?.strip_suffix('>')?;
        if !pieces.any(|p| p == wanted) {
            return None;
        }
        Url::parse(url)
            .ok()?
            .query_pairs()
            .find(|(k, _)| k == "page")?
            .1
            .parse()
            .ok()
    })
}

fn extract_repo_name(repo_url: &str) -> &str {
    match repo_url.split_once("/repos/") {
        Some((_, name)) => name,
        None => repo_url,
    }
}

fn average(durations: impl Iterator<Item = TimeDelta>) -> TimeDelta {
    let (total, count) = durations.fold((TimeDelta::zero(), 0i32), |(sum, n), d| (sum + d, n + 1));
    if count == 0 {
        TimeDelta::zero()
    } else {
        total / count
    }
}

/// The `limit` repositories with the highest counts, ties broken by name.
pub fn top_repos(counts: HashMap<String, u64>, limit: usize) -> Vec<RepoCount> {
    let mut repos: Vec<RepoCount> = counts
        .into_iter()
        .map(|(repo_name, count)| RepoCount { repo_name, count })
        .collect();
    repos.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.repo_name.cmp(&b.repo_name))
    });
    repos.truncate(limit);
    repos
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_link_header() {
        let link = r#"<https://api.github.com/user/repos?per_page=1&page=2>; rel="next", <https://api.github.com/user/repos?per_page=1&page=37>; rel="last""#;
        assert_eq!(link_page(link, "next"), Some(2));
        assert_eq!(link_page(link, "last"), Some(37));
        assert_eq!(link_page(link, "prev"), None);
        assert_eq!(link_page("", "next"), None);
    }

    #[test]
    fn extracts_repo_name() {
        assert_eq!(
            extract_repo_name("https://api.github.com/repos/rust-lang/rust"),
            "rust-lang/rust"
        );
        assert_eq!(extract_repo_name("weird"), "weird");
    }

    #[test]
    fn top_repos_sorted_and_limited() {
        let counts = HashMap::from([
            ("b".to_string(), 3),
            ("a".to_string(), 3),
            ("c".to_string(), 7),
            ("d".to_string(), 1),
        ]);
        let top = top_repos(counts, 3);
        let names: Vec<_> = top.iter().map(|r| r.repo_name.as_str()).collect();
        assert_eq!(names, ["c", "a", "b"]);
    }

    #[test]
    fn average_of_durations() {
        let avg = average([TimeDelta::hours(2), TimeDelta::hours(4)].into_iter());
        assert_eq!(avg, TimeDelta::hours(3));
        assert_eq!(average(std::iter::empty()), TimeDelta::zero());
    }
}

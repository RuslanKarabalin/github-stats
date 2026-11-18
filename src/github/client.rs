use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Datelike, Local, NaiveDate, SecondsFormat, TimeDelta, TimeZone, Utc};
use futures::{StreamExt, stream};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, LINK, USER_AGENT};
use reqwest::{Method, RequestBuilder, Response, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::types::{
    ContributionCalendar, ContributionDay, ContributionYear, IssueStats, OpenPullRequest,
    PullRequestStats, RepoCount,
};

const API_URL: &str = "https://api.github.com";
const GRAPHQL_URL: &str = "https://api.github.com/graphql";
const PER_PAGE: u32 = 100;
const EVENT_PAGES: u32 = 3;
const SEARCH_LIMIT: usize = 1000;
const GRAPHQL_CONCURRENCY: usize = 2;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ATTEMPTS: u32 = 4;
const MAX_RETRY_WAIT: Duration = Duration::from_secs(90);
const SECONDARY_LIMIT_WAIT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    api_url: String,
    graphql_url: String,
    max_workers: usize,
}

#[derive(Debug)]
pub struct ApiError {
    pub method: Method,
    pub url: String,
    pub status: StatusCode,
    pub message: String,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: {}", self.method, self.url, self.status)?;
        if !self.message.is_empty() {
            write!(f, " {}", self.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {}

#[derive(Debug, Default)]
pub struct Failures {
    count: usize,
    first: Option<anyhow::Error>,
}

impl Failures {
    fn record(&mut self, err: anyhow::Error) {
        self.count += 1;
        self.first.get_or_insert(err);
    }

    pub fn warning(&self, what: &str) -> Option<String> {
        let first = self.first.as_ref()?;
        Some(if self.count == 1 {
            format!("{what}: {first:#}")
        } else {
            format!(
                "{what}: {} repositories failed, the first with: {first:#}",
                self.count
            )
        })
    }
}

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
    total_count: u64,
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
    pull_request: Option<ApiIssuePullRequest>,
}

#[derive(Debug, Deserialize)]
struct ApiIssuePullRequest {
    merged_at: Option<DateTime<Utc>>,
}

impl ApiIssue {
    fn is_open(&self) -> bool {
        self.state.as_deref() == Some("open")
    }

    fn merged_at(&self) -> Option<DateTime<Utc>> {
        self.pull_request.as_ref()?.merged_at
    }
}

struct Search {
    total: u64,
    items: Vec<ApiIssue>,
}

impl Search {
    fn from_items(items: Vec<ApiIssue>) -> Self {
        Self {
            total: items.len() as u64,
            items,
        }
    }

    fn complete(&self) -> bool {
        self.items.len() as u64 >= self.total
    }
}

#[derive(Debug, Deserialize)]
struct RateLimitResponse {
    resources: RateLimitResources,
}

#[derive(Debug, Deserialize)]
struct RateLimitResources {
    core: Option<ApiRate>,
    graphql: Option<ApiRate>,
    search: Option<ApiRate>,
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

impl From<ApiRate> for Rate {
    fn from(rate: ApiRate) -> Self {
        Self {
            limit: rate.limit,
            remaining: rate.remaining,
            reset: Local
                .timestamp_opt(rate.reset, 0)
                .single()
                .unwrap_or_else(Local::now),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RateLimits {
    pub core: Option<Rate>,
    pub graphql: Option<Rate>,
    pub search: Option<Rate>,
}

#[derive(Serialize)]
struct GraphQlRequest<'a> {
    query: &'a str,
    variables: serde_json::Value,
}

#[derive(Deserialize)]
struct GraphQlResponse {
    data: Option<serde_json::Value>,
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
    #[serde(default)]
    pull_request_review_contributions_by_repository: Vec<RepoContributions>,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoContributions {
    repository: RepoName,
    contributions: ContributionCount,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepoName {
    name_with_owner: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContributionCount {
    total_count: u64,
}

struct YearContributions {
    days: Vec<ContributionDay>,
    summary: ContributionYear,
    reviews_by_repo: Vec<(String, u64)>,
    warnings: Vec<String>,
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
      pullRequestReviewContributionsByRepository(maxRepositories: 100) {
        repository {
          nameWithOwner
        }
        contributions(first: 1) {
          totalCount
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
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            http,
            api_url: API_URL.to_string(),
            graphql_url: GRAPHQL_URL.to_string(),
            max_workers: max_workers.max(1),
        })
    }

    #[cfg(test)]
    fn with_base_url(mut self, base: &str) -> Self {
        self.api_url = base.trim_end_matches('/').to_string();
        self.graphql_url = format!("{}/graphql", self.api_url);
        self
    }

    async fn send(&self, request: impl Fn() -> RequestBuilder) -> Result<Response> {
        let mut attempt = 1;
        loop {
            let req = request().build().context("failed to build request")?;
            let method = req.method().clone();
            let url = req.url().to_string();

            let resp = match self.http.execute(req).await {
                Ok(resp) => resp,
                Err(err) if attempt < MAX_ATTEMPTS && (err.is_timeout() || err.is_connect()) => {
                    tokio::time::sleep(backoff(attempt)).await;
                    attempt += 1;
                    continue;
                }
                Err(err) => {
                    return Err(anyhow::Error::new(err).context(format!("{method} {url} failed")));
                }
            };

            let status = resp.status();
            if status.is_success() {
                return Ok(resp);
            }

            let headers = resp.headers().clone();
            let body = resp.text().await.unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("message")?.as_str().map(str::to_owned))
                .unwrap_or_default();

            if attempt < MAX_ATTEMPTS
                && let Some(wait) = retry_delay(status, &headers, &message, attempt, Utc::now())
                && wait <= MAX_RETRY_WAIT
            {
                tokio::time::sleep(wait).await;
                attempt += 1;
                continue;
            }

            return Err(ApiError {
                method,
                url,
                status,
                message,
            }
            .into());
        }
    }

    async fn get_page<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Page<T>> {
        let url = format!("{}{path}", self.api_url);
        let resp = self.send(|| self.http.get(&url).query(query)).await?;
        let link = resp
            .headers()
            .get(LINK)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let full_url = resp.url().to_string();

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

    async fn graphql<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<(Option<T>, Vec<String>)> {
        let request = GraphQlRequest { query, variables };
        let resp = self
            .send(|| self.http.post(&self.graphql_url).json(&request))
            .await
            .context("GraphQL request failed")?;

        let body: GraphQlResponse = resp
            .json()
            .await
            .context("failed to parse GraphQL response")?;
        let errors: Vec<String> = body.errors.into_iter().map(|e| e.message).collect();

        let data = match body.data {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => match serde_json::from_value::<T>(value) {
                Ok(data) => Some(data),
                Err(_) if !errors.is_empty() => bail!("GraphQL error: {}", errors[0]),
                Err(err) => return Err(err).context("failed to decode GraphQL data"),
            },
        };
        if data.is_none()
            && let Some(message) = errors.first()
        {
            bail!("GraphQL error: {message}");
        }

        Ok((data, errors))
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
            Err(err) if has_status(&err, StatusCode::NOT_FOUND) => {
                Err(anyhow!("user '{username}' not found"))
            }
            Err(err) => Err(err.context("failed to get user")),
        }
    }

    pub async fn repositories(
        &self,
        username: &str,
        include_private: bool,
    ) -> Result<Vec<ApiRepo>> {
        let result = if include_private {
            let query = [
                ("affiliation", "owner".to_string()),
                ("visibility", "all".to_string()),
                ("sort", "updated".to_string()),
            ];
            self.get_all("/user/repos", &query).await
        } else {
            let query = [
                ("type", "owner".to_string()),
                ("sort", "updated".to_string()),
            ];
            self.get_all(&format!("/users/{username}/repos"), &query)
                .await
        };
        result.context("failed to list repositories")
    }

    pub async fn languages(&self, repos: &[ApiRepo]) -> (BTreeMap<String, u64>, Failures) {
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
        let mut failures = Failures::default();
        for result in results {
            match result {
                Ok(langs) => {
                    for (lang, bytes) in langs {
                        *languages.entry(lang).or_default() += bytes;
                    }
                }
                Err(err) => failures.record(err),
            }
        }

        (languages, failures)
    }

    pub async fn commit_activity(
        &self,
        username: &str,
        repos: &[ApiRepo],
        full_scan: bool,
    ) -> Result<(Vec<DateTime<Utc>>, Failures)> {
        if full_scan {
            Ok(self.commit_activity_full(username, repos).await)
        } else {
            let times = self.commit_activity_recent(username).await?;
            Ok((times, Failures::default()))
        }
    }

    async fn commit_activity_recent(&self, username: &str) -> Result<Vec<DateTime<Utc>>> {
        let mut times = Vec::new();

        for page in 1..=EVENT_PAGES {
            let query = [
                ("per_page", PER_PAGE.to_string()),
                ("page", page.to_string()),
            ];
            let resp: Page<Vec<ApiEvent>> = self
                .get_page(&format!("/users/{username}/events"), &query)
                .await
                .context("failed to list events")?;

            times.extend(
                resp.items
                    .into_iter()
                    .filter(|e| e.kind.as_deref() == Some("PushEvent"))
                    .filter_map(|e| e.created_at),
            );

            if resp.next.is_none() {
                break;
            }
        }

        Ok(times)
    }

    async fn commit_activity_full(
        &self,
        username: &str,
        repos: &[ApiRepo],
    ) -> (Vec<DateTime<Utc>>, Failures) {
        let per_repo: Vec<_> = stream::iter(repos)
            .map(|repo| self.repo_commit_dates(username, &repo.owner.login, &repo.name))
            .buffer_unordered(self.max_workers)
            .collect()
            .await;

        let mut times = Vec::new();
        let mut failures = Failures::default();
        for (dates, err) in per_repo {
            times.extend(dates);
            if let Some(err) = err {
                failures.record(err);
            }
        }
        (times, failures)
    }

    async fn repo_commit_dates(
        &self,
        author: &str,
        owner: &str,
        repo: &str,
    ) -> (Vec<DateTime<Utc>>, Option<anyhow::Error>) {
        let path = format!("/repos/{owner}/{repo}/commits");
        let mut dates = Vec::new();
        let mut page = 1;

        loop {
            let query = [
                ("author", author.to_string()),
                ("per_page", PER_PAGE.to_string()),
                ("page", page.to_string()),
            ];
            let resp = match self.get_page::<Vec<ApiCommit>>(&path, &query).await {
                Ok(resp) => resp,
                Err(err) if has_status(&err, StatusCode::CONFLICT) => return (dates, None),
                Err(err) => {
                    let err = err.context(format!("failed to list commits of {owner}/{repo}"));
                    return (dates, Some(err));
                }
            };

            dates.extend(
                resp.items
                    .into_iter()
                    .filter_map(|c| c.commit?.author?.date),
            );

            match resp.next {
                Some(next) => page = next,
                None => return (dates, None),
            }
        }
    }

    pub async fn top_repos_by_commits(
        &self,
        username: &str,
        repos: &[ApiRepo],
        limit: usize,
    ) -> (Vec<RepoCount>, Failures) {
        let results: Vec<_> = stream::iter(repos.iter().filter(|r| !r.fork))
            .map(|repo| async move {
                let count = self
                    .count_repo_commits(username, &repo.owner.login, &repo.name)
                    .await;
                (format!("{}/{}", repo.owner.login, repo.name), count)
            })
            .buffer_unordered(self.max_workers)
            .collect()
            .await;

        let mut counts = HashMap::new();
        let mut failures = Failures::default();
        for (name, result) in results {
            match result {
                Ok(0) => {}
                Ok(count) => {
                    counts.insert(name, count);
                }
                Err(err) => failures.record(err),
            }
        }

        (top_repos(counts, limit), failures)
    }

    async fn count_repo_commits(&self, author: &str, owner: &str, repo: &str) -> Result<u64> {
        let query = [
            ("author", author.to_string()),
            ("per_page", "1".to_string()),
        ];
        match self
            .get_page::<Vec<serde_json::Value>>(&format!("/repos/{owner}/{repo}/commits"), &query)
            .await
        {
            Ok(page) => Ok(page.last.map_or(page.items.len() as u64, u64::from)),
            Err(err) if has_status(&err, StatusCode::CONFLICT) => Ok(0),
            Err(err) => Err(err.context(format!("failed to count commits of {owner}/{repo}"))),
        }
    }

    pub async fn contribution_calendar(
        &self,
        username: &str,
        since: Option<DateTime<Utc>>,
    ) -> Result<ContributionCalendar> {
        let now = Utc::now();
        let first_year = since.map_or(now.year(), |s| s.year()).min(now.year());

        let results: Vec<(i32, Result<YearContributions>)> =
            stream::iter((first_year..=now.year()).rev())
                .map(|year| async move {
                    let from = Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).unwrap();
                    let to = Utc
                        .with_ymd_and_hms(year, 12, 31, 23, 59, 59)
                        .unwrap()
                        .min(now);
                    let result = self.contributions_for_period(username, from, to).await;
                    (
                        year,
                        result.map(|mut c| {
                            c.summary.year = year;
                            c
                        }),
                    )
                })
                .buffered(self.max_workers.min(GRAPHQL_CONCURRENCY))
                .collect()
                .await;

        let mut calendar = ContributionCalendar::default();
        let mut collected = Vec::new();
        for (i, (year, result)) in results.into_iter().enumerate() {
            match result {
                Ok(c) => collected.push(c),
                Err(err) if i == 0 => return Err(err),
                Err(err) => {
                    calendar.warnings.push(format!(
                        "contribution history before {} is missing: {err:#}",
                        year + 1
                    ));
                    break;
                }
            }
        }
        collected.reverse();

        for c in collected {
            calendar.days.extend(c.days);
            calendar.years.push(c.summary);
            for (repo, count) in c.reviews_by_repo {
                *calendar.reviews_by_repo.entry(repo).or_default() += count;
            }
            calendar.warnings.extend(c.warnings);
        }

        Ok(calendar)
    }

    async fn contributions_for_period(
        &self,
        username: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<YearContributions> {
        let variables = json!({
            "username": username,
            "from": from.to_rfc3339_opts(SecondsFormat::Secs, true),
            "to": to.to_rfc3339_opts(SecondsFormat::Secs, true),
        });

        let (data, errors): (Option<UserData<CalendarCollection>>, _) =
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

        let reviews_by_repo = c
            .pull_request_review_contributions_by_repository
            .into_iter()
            .map(|r| (r.repository.name_with_owner, r.contributions.total_count))
            .collect();

        let warnings = errors
            .into_iter()
            .map(|e| format!("contributions for {}: {e}", from.year()))
            .collect();

        Ok(YearContributions {
            days,
            summary,
            reviews_by_repo,
            warnings,
        })
    }

    pub async fn rate_limit(&self) -> Result<RateLimits> {
        let resp: RateLimitResponse = self
            .get("/rate_limit")
            .await
            .context("failed to check rate limit")?;

        Ok(RateLimits {
            core: resp.resources.core.map(Rate::from),
            graphql: resp.resources.graphql.map(Rate::from),
            search: resp.resources.search.map(Rate::from),
        })
    }

    async fn search_issues(&self, query: &str) -> Result<Search> {
        let mut items = Vec::new();
        let mut total = 0;
        let mut page = 1;
        loop {
            let q = [
                ("q", query.to_string()),
                ("sort", "created".to_string()),
                ("per_page", PER_PAGE.to_string()),
                ("page", page.to_string()),
            ];
            let resp: Page<SearchResult> = self.get_page("/search/issues", &q).await?;
            if page == 1 {
                total = resp.items.total_count;
            }
            items.extend(resp.items.items);

            match resp.next {
                Some(next) if items.len() < SEARCH_LIMIT => page = next,
                _ => {
                    return Ok(Search {
                        total: total.max(items.len() as u64),
                        items,
                    });
                }
            }
        }
    }

    async fn search_count(&self, query: &str) -> Result<u64> {
        let q = [("q", query.to_string()), ("per_page", "1".to_string())];
        let resp: Page<SearchResult> = self.get_page("/search/issues", &q).await?;
        Ok(resp.items.total_count)
    }

    pub async fn pull_requests(&self, username: &str) -> Result<(PullRequestStats, Vec<String>)> {
        let mut warnings = Vec::new();
        let all = self
            .search_issues(&format!("author:{username} is:pr"))
            .await
            .context("failed to search PRs")?;

        let mut stats = PullRequestStats {
            total: all.total,
            ..PullRequestStats::default()
        };

        let mut repo_count: HashMap<String, u64> = HashMap::new();
        for pr in &all.items {
            if let Some(url) = &pr.repository_url {
                *repo_count
                    .entry(extract_repo_name(url).to_string())
                    .or_default() += 1;
            }
        }
        stats.top_repos = top_repos(repo_count, 5);

        let (open, merged) = if all.complete() {
            let (open, rest): (Vec<_>, Vec<_>) = all.items.into_iter().partition(ApiIssue::is_open);
            let merged = rest
                .into_iter()
                .filter(|pr| pr.merged_at().is_some())
                .collect();
            (Search::from_items(open), Search::from_items(merged))
        } else {
            let seen = all.items.len();
            warnings.push(format!(
                "the search API returned only the latest {seen} of {} pull requests; \
                 top repositories by PRs are based on them",
                all.total
            ));
            let open = self
                .search_issues(&format!("author:{username} is:pr is:open"))
                .await
                .context("failed to search open PRs")?;
            let merged_total = self
                .search_count(&format!("author:{username} is:pr is:merged"))
                .await
                .context("failed to count merged PRs")?;
            let merged: Vec<ApiIssue> = all
                .items
                .into_iter()
                .filter(|pr| pr.merged_at().is_some())
                .collect();
            warnings.push(format!(
                "average merge time is based on the {} merged among the latest {seen} pull requests",
                merged.len()
            ));
            (
                open,
                Search {
                    total: merged_total,
                    items: merged,
                },
            )
        };

        stats.open = open.total;
        stats.merged = merged.total;
        stats.closed = stats.total.saturating_sub(stats.open + stats.merged);
        stats.avg_merge_time = average(
            merged
                .items
                .iter()
                .filter_map(|pr| Some(pr.merged_at()? - pr.created_at?)),
        );
        stats.open_prs = open
            .items
            .into_iter()
            .map(|pr| OpenPullRequest {
                repo_name: pr
                    .repository_url
                    .as_deref()
                    .map(extract_repo_name)
                    .unwrap_or_default()
                    .to_string(),
                title: pr.title.unwrap_or_default(),
                number: pr.number.unwrap_or_default(),
                created_at: pr.created_at,
                url: pr.html_url.unwrap_or_default(),
            })
            .collect();

        Ok((stats, warnings))
    }

    pub async fn issues(&self, username: &str) -> Result<(IssueStats, Vec<String>)> {
        let mut warnings = Vec::new();
        let all = self
            .search_issues(&format!("author:{username} is:issue"))
            .await
            .context("failed to search issues")?;

        let open = if all.complete() {
            all.items.iter().filter(|i| i.is_open()).count() as u64
        } else {
            warnings.push(format!(
                "average close time is based on the latest {} of {} issues",
                all.items.len(),
                all.total
            ));
            self.search_count(&format!("author:{username} is:issue is:open"))
                .await
                .context("failed to count open issues")?
        };

        let close_times = all
            .items
            .iter()
            .filter(|i| i.state.as_deref() == Some("closed"))
            .filter_map(|i| Some(i.closed_at? - i.created_at?));

        let stats = IssueStats {
            total: all.total,
            open,
            closed: all.total.saturating_sub(open),
            avg_close_time: average(close_times),
        };
        Ok((stats, warnings))
    }
}

fn has_status(err: &anyhow::Error, status: StatusCode) -> bool {
    err.downcast_ref::<ApiError>()
        .is_some_and(|e| e.status == status)
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1 << attempt.min(5))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok().map(str::trim)
}

fn retry_delay(
    status: StatusCode,
    headers: &HeaderMap,
    message: &str,
    attempt: u32,
    now: DateTime<Utc>,
) -> Option<Duration> {
    let retry_after = header(headers, "retry-after").and_then(|v| v.parse::<u64>().ok());
    let exhausted = header(headers, "x-ratelimit-remaining") == Some("0");
    let rate_limited = status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::FORBIDDEN
            && (retry_after.is_some()
                || exhausted
                || message.to_ascii_lowercase().contains("rate limit")));

    if rate_limited {
        if let Some(secs) = retry_after {
            return Some(Duration::from_secs(secs));
        }
        if exhausted
            && let Some(reset) =
                header(headers, "x-ratelimit-reset").and_then(|v| v.parse::<i64>().ok())
        {
            let secs = (reset - now.timestamp()).max(0).unsigned_abs() + 1;
            return Some(Duration::from_secs(secs));
        }
        return Some(SECONDARY_LIMIT_WAIT);
    }

    if matches!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    ) {
        return Some(backoff(attempt));
    }

    None
}

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
pub(crate) async fn mock_client(app: axum::Router) -> Client {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Client::new("test-token", 4).unwrap().with_base_url(&base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::extract::Query;
    use axum::response::IntoResponse;
    use axum::routing::get;

    fn repo_json(name: &str) -> serde_json::Value {
        json!({ "name": name, "owner": { "login": "octocat" } })
    }

    #[tokio::test]
    async fn get_all_follows_next_links() {
        let app = axum::Router::new().route(
            "/users/octocat/repos",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                if q.get("page").map(String::as_str) == Some("2") {
                    Json(json!([repo_json("b")])).into_response()
                } else {
                    (
                        [("link", r#"<http://x/?page=2>; rel="next""#)],
                        Json(json!([repo_json("a")])),
                    )
                        .into_response()
                }
            }),
        );
        let client = mock_client(app).await;
        let repos = client.repositories("octocat", false).await.unwrap();
        let names: Vec<_> = repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[tokio::test]
    async fn counts_commits_from_last_page_link() {
        let app = axum::Router::new()
            .route(
                "/repos/octocat/busy/commits",
                get(|| async {
                    (
                        [("link", r#"<http://x/?per_page=1&page=42>; rel="last""#)],
                        Json(json!([{}])),
                    )
                }),
            )
            .route(
                "/repos/octocat/empty/commits",
                get(|| async {
                    (
                        StatusCode::CONFLICT,
                        Json(json!({ "message": "Git Repository is empty." })),
                    )
                }),
            );
        let client = mock_client(app).await;
        let count = |repo: &'static str| client.count_repo_commits("octocat", "octocat", repo);
        assert_eq!(count("busy").await.unwrap(), 42);
        assert_eq!(count("empty").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn many_prs_are_counted_without_listing_merged_ones() {
        let pr = |state: &str, merged: Option<&str>| {
            json!({
                "repository_url": "https://api.github.com/repos/o/r",
                "state": state,
                "created_at": "2026-01-01T00:00:00Z",
                "pull_request": { "merged_at": merged },
            })
        };
        let all = json!({
            "total_count": 1500,
            "items": [
                pr("open", None),
                pr("closed", Some("2026-01-01T02:00:00Z")),
                pr("closed", None),
            ],
        });
        let open = json!({ "total_count": 1, "items": [pr("open", None)] });
        let merged = json!({ "total_count": 900, "items": [] });
        let app = axum::Router::new().route(
            "/search/issues",
            get(move |Query(q): Query<HashMap<String, String>>| async move {
                match q["q"].as_str() {
                    "author:octocat is:pr" => Json(all),
                    "author:octocat is:pr is:open" => Json(open),
                    "author:octocat is:pr is:merged" => Json(merged),
                    other => panic!("unexpected search {other}"),
                }
            }),
        );

        let client = mock_client(app).await;
        let (stats, warnings) = client.pull_requests("octocat").await.unwrap();
        assert_eq!(
            (stats.total, stats.open, stats.merged, stats.closed),
            (1500, 1, 900, 599)
        );
        assert_eq!(stats.avg_merge_time, TimeDelta::hours(2));
        assert_eq!(stats.open_prs.len(), 1);
        assert_eq!(stats.top_repos[0].repo_name, "o/r");
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[tokio::test]
    async fn missing_user_is_reported_by_name() {
        let client = mock_client(axum::Router::new()).await;
        let err = client.user("ghost").await.unwrap_err();
        assert_eq!(err.to_string(), "user 'ghost' not found");
    }

    #[tokio::test]
    async fn graphql_errors_with_data_become_warnings() {
        let app = axum::Router::new().route(
            "/graphql",
            axum::routing::post(|| async {
                Json(json!({ "data": { "n": 1 }, "errors": [{ "message": "partial" }] }))
            }),
        );
        let client = mock_client(app).await;
        let (data, errors) = client
            .graphql::<serde_json::Value>("{ n }", json!({}))
            .await
            .unwrap();
        assert_eq!(data.unwrap()["n"], 1);
        assert_eq!(errors, ["partial"]);
    }

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

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn retry_delay_for_rate_limits() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let delay =
            |status, h: &[(&'static str, &str)], msg| retry_delay(status, &headers(h), msg, 1, now);

        assert_eq!(
            delay(StatusCode::FORBIDDEN, &[("retry-after", "7")], ""),
            Some(Duration::from_secs(7))
        );
        let reset = (now.timestamp() + 30).to_string();
        assert_eq!(
            delay(
                StatusCode::FORBIDDEN,
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", &reset)
                ],
                ""
            ),
            Some(Duration::from_secs(31))
        );
        assert_eq!(
            delay(
                StatusCode::FORBIDDEN,
                &[],
                "You have exceeded a secondary rate limit"
            ),
            Some(SECONDARY_LIMIT_WAIT)
        );
        assert_eq!(
            delay(StatusCode::TOO_MANY_REQUESTS, &[], ""),
            Some(SECONDARY_LIMIT_WAIT)
        );
    }

    #[test]
    fn retry_delay_for_other_errors() {
        let now = Utc::now();
        let none = HeaderMap::new();
        assert_eq!(
            retry_delay(StatusCode::BAD_GATEWAY, &none, "", 2, now),
            Some(Duration::from_secs(4))
        );
        assert_eq!(
            retry_delay(
                StatusCode::FORBIDDEN,
                &none,
                "Resource not accessible",
                1,
                now
            ),
            None
        );
        assert_eq!(retry_delay(StatusCode::NOT_FOUND, &none, "", 1, now), None);
    }

    #[test]
    fn failures_summarize_first_error() {
        let mut failures = Failures::default();
        assert_eq!(failures.warning("x"), None);
        failures.record(anyhow!("boom"));
        assert_eq!(failures.warning("lang").unwrap(), "lang: boom");
        failures.record(anyhow!("later"));
        assert_eq!(
            failures.warning("lang").unwrap(),
            "lang: 2 repositories failed, the first with: boom"
        );
    }

    #[test]
    fn search_items_know_merge_state() {
        let item: ApiIssue = serde_json::from_value(json!({
            "state": "closed",
            "pull_request": { "merged_at": "2026-01-02T00:00:00Z" }
        }))
        .unwrap();
        assert!(!item.is_open());
        assert!(item.merged_at().is_some());
    }
}

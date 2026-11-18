mod charts;

use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Bytes;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::any;
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeDelta};
use minijinja::value::Serde;
use minijinja::{Environment, UndefinedBehavior};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::format;
use crate::github::{PullRequestStats, RepoCount, UserStats, language_stats};
use charts::{build_chart_data, summarize_contributions};

const PAGE_TEMPLATE: &str = include_str!("page.html");
const CHARTS_SCRIPT: &str = include_str!("charts.js");
const MAX_LANGUAGE_SLOTS: usize = 8;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Server {
    listener: TcpListener,
    addr: SocketAddr,
    page: Bytes,
}

impl Server {
    pub async fn new(stats: &UserStats, host: &str, port: u16) -> Result<Self> {
        let page = render(stats, Local::now())?;

        let address = format!("{host}:{port}");
        let listener = TcpListener::bind(&address)
            .await
            .with_context(|| format!("failed to listen on {address}"))?;
        let addr = listener.local_addr()?;

        Ok(Self {
            listener,
            addr,
            page: page.into(),
        })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) -> Result<()> {
        let page = self.page;
        let addr = self.addr;
        let app = Router::new()
            .route("/", any(move || async move { Html(page) }))
            .fallback(|| async { (StatusCode::NOT_FOUND, "404 page not found\n") })
            .layer(middleware::from_fn(move |req: Request, next: Next| {
                check_host(addr, req, next)
            }));

        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let serve = axum::serve(self.listener, app)
            .with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .into_future();
        let mut serve = std::pin::pin!(serve);

        tokio::select! {
            result = &mut serve => return result.context("server failed"),
            () = shutdown => {}
        }

        let _ = stop_tx.send(());
        match tokio::time::timeout(SHUTDOWN_TIMEOUT, serve).await {
            Ok(result) => result.context("server failed"),
            Err(_) => Ok(()),
        }
    }
}

async fn check_host(addr: SocketAddr, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok());
    if host.is_some_and(|host| is_local_host(host, addr)) {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "403 unexpected Host header\n").into_response()
    }
}

fn is_local_host(host: &str, addr: SocketAddr) -> bool {
    let host = host.trim().to_ascii_lowercase();
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) if !host.ends_with(']') => match port.parse::<u16>() {
            Ok(port) => (name, port),
            Err(_) => return false,
        },
        _ => (host.as_str(), 80),
    };
    let bound = match addr {
        SocketAddr::V4(a) => a.ip().to_string(),
        SocketAddr::V6(a) => format!("[{}]", a.ip()),
    };
    port == addr.port() && (matches!(name, "localhost" | "127.0.0.1" | "[::1]") || name == bound)
}

#[derive(Serialize)]
struct PageContext {
    username: String,
    display_name: String,
    avatar_url: String,
    bio: String,
    company: String,
    location: String,
    blog: String,
    blog_href: String,
    joined: String,
    account_age: String,
    includes_private_repos: bool,
    followers: String,
    following: String,
    public_repos: String,
    total_repos: String,
    public_gists: String,
    total_stars: String,
    total_forks: String,
    contrib: Option<ContribView>,
    streak: StreakView,
    charts: ChartFlags,
    languages: Vec<Segment>,
    top_repositories: Vec<RepoRow>,
    commit_bars: Vec<Bar>,
    prs: Option<PrView>,
    issues: Option<IssueView>,
    reviews: Option<ReviewView>,
    chart_json: String,
    charts_script: &'static str,
    generated_at: String,
}

#[derive(Serialize)]
struct ContribView {
    total: String,
    this_year: String,
    best_count: String,
    best_date: String,
    active_avg: String,
}

#[derive(Serialize)]
struct StreakView {
    max: String,
    max_unit: &'static str,
    max_period: Option<String>,
    current: String,
    current_since: Option<String>,
    active_days: String,
    most_active_day: Option<String>,
    most_active_hour: Option<String>,
}

#[derive(Serialize)]
struct ChartFlags {
    monthly: bool,
    years: bool,
    weekdays: bool,
    repos: bool,
}

#[derive(Debug, Serialize)]
struct Segment {
    name: String,
    pct: f64,
    pct_label: String,
    detail: String,
    slot: usize,
    style: String,
}

#[derive(Debug, Serialize)]
struct Bar {
    label: String,
    value: String,
    style: String,
}

#[derive(Serialize)]
struct RepoRow {
    name: String,
    language: String,
    stars: String,
    forks: String,
}

#[derive(Serialize)]
struct PrView {
    total: String,
    open: String,
    merged: String,
    closed: String,
    avg_merge_time: Option<String>,
    outcomes: Vec<Segment>,
    bars: Vec<Bar>,
    open_prs: Vec<OpenPrRow>,
}

#[derive(Serialize)]
struct OpenPrRow {
    repo_name: String,
    url: String,
    number: u64,
    title: String,
    opened: String,
}

#[derive(Serialize)]
struct IssueView {
    total: String,
    open: String,
    closed: String,
    avg_close_time: Option<String>,
}

#[derive(Serialize)]
struct ReviewView {
    total: String,
    bars: Vec<Bar>,
}

fn render(stats: &UserStats, now: DateTime<Local>) -> Result<String> {
    let charts = build_chart_data(stats, now.to_utc().date_naive());

    let ctx = PageContext {
        username: stats.username.clone(),
        display_name: if stats.name.is_empty() {
            stats.username.clone()
        } else {
            stats.name.clone()
        },
        avatar_url: safe_url(&stats.avatar_url),
        bio: stats.bio.clone(),
        company: stats.company.clone(),
        location: stats.location.clone(),
        blog: stats.blog.clone(),
        blog_href: external_url(&stats.blog),
        joined: human_date(stats.created_at.map(|t| t.date_naive())),
        account_age: stats.account_age.to_string(),
        includes_private_repos: stats.includes_private_repos,
        followers: format::count(stats.followers),
        following: format::count(stats.following),
        public_repos: format::count(stats.public_repos),
        total_repos: format::count(stats.total_repos),
        public_gists: format::count(stats.public_gists),
        total_stars: format::count(stats.total_stars),
        total_forks: format::count(stats.total_forks),
        contrib: summarize_contributions(&stats.contributions, now.to_utc().date_naive()).map(
            |c| ContribView {
                total: format::count(c.total),
                this_year: format::count(c.this_year),
                best_count: format::count(c.best_count),
                best_date: human_date(Some(c.best_date)),
                active_avg: format!("{:.1}", c.active_avg),
            },
        ),
        streak: StreakView {
            max: format::count(stats.max_streak),
            max_unit: if stats.max_streak == 1 { "day" } else { "days" },
            max_period: stats.max_streak_start.map(|start| {
                format!(
                    "{} \u{2014} {}",
                    human_date(Some(start)),
                    human_date(stats.max_streak_end)
                )
            }),
            current: format::count(stats.current_streak),
            current_since: (stats.current_streak > 0)
                .then(|| human_date(stats.current_streak_start)),
            active_days: format::count(stats.total_active_days),
            most_active_day: stats.most_active_day.clone(),
            most_active_hour: stats.most_active_hour.map(format::hour),
        },
        charts: ChartFlags {
            monthly: !charts.monthly.is_empty(),
            years: !charts.years.is_empty(),
            weekdays: !charts.weekdays.is_empty(),
            repos: !charts.repos.is_empty(),
        },
        languages: language_segments(stats),
        top_repositories: stats
            .top_repositories
            .iter()
            .map(|r| RepoRow {
                name: r.name.clone(),
                language: if r.language.is_empty() {
                    "\u{2014}".to_string()
                } else {
                    r.language.clone()
                },
                stars: format::count(r.stars),
                forks: format::count(r.forks),
            })
            .collect(),
        commit_bars: bars(&stats.top_repos_by_commits),
        prs: stats
            .pr_stats
            .as_ref()
            .filter(|p| p.total > 0)
            .map(|p| PrView {
                total: format::count(p.total),
                open: format::count(p.open),
                merged: format::count(p.merged),
                closed: format::count(p.closed),
                avg_merge_time: positive_duration(p.avg_merge_time),
                outcomes: pr_outcome_segments(p),
                bars: bars(&p.top_repos),
                open_prs: p
                    .open_prs
                    .iter()
                    .map(|pr| OpenPrRow {
                        repo_name: pr.repo_name.clone(),
                        url: safe_url(&pr.url),
                        number: pr.number,
                        title: pr.title.clone(),
                        opened: human_date(pr.created_at.map(|t| t.date_naive())),
                    })
                    .collect(),
            }),
        issues: stats
            .issue_stats
            .as_ref()
            .filter(|i| i.total > 0)
            .map(|i| IssueView {
                total: format::count(i.total),
                open: format::count(i.open),
                closed: format::count(i.closed),
                avg_close_time: positive_duration(i.avg_close_time),
            }),
        reviews: stats
            .review_stats
            .as_ref()
            .filter(|r| r.total > 0)
            .map(|r| ReviewView {
                total: format::count(r.total),
                bars: bars(&r.top_repos),
            }),
        chart_json: script_safe_json(&charts)?,
        charts_script: CHARTS_SCRIPT,
        generated_at: now.format("%Y-%m-%d %H:%M:%S %Z").to_string(),
    };

    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.add_template("page.html", PAGE_TEMPLATE)
        .context("failed to parse page template")?;
    env.get_template("page.html")?
        .render(Serde(&ctx))
        .context("failed to render page")
}

fn language_segments(stats: &UserStats) -> Vec<Segment> {
    let langs = language_stats(&stats.languages);
    let mut segments: Vec<Segment> = langs
        .iter()
        .take(MAX_LANGUAGE_SLOTS)
        .enumerate()
        .map(|(i, l)| segment(&l.name, l.percentage, format::bytes(l.bytes), i + 1))
        .collect();

    let rest = &langs[langs.len().min(MAX_LANGUAGE_SLOTS)..];
    let other_bytes: u64 = rest.iter().map(|l| l.bytes).sum();
    if other_bytes > 0 {
        let other_pct = rest.iter().map(|l| l.percentage).sum();
        segments.push(segment("Other", other_pct, format::bytes(other_bytes), 0));
    }

    segments
}

fn pr_outcome_segments(prs: &PullRequestStats) -> Vec<Segment> {
    let total = prs.merged + prs.open + prs.closed;
    if total == 0 {
        return Vec::new();
    }

    [
        ("Merged", prs.merged),
        ("Open", prs.open),
        ("Closed unmerged", prs.closed),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (name, count))| {
        let pct = count as f64 / total as f64 * 100.0;
        segment(name, pct, format::count(count), i + 1)
    })
    .collect()
}

fn segment(name: &str, pct: f64, detail: String, slot: usize) -> Segment {
    Segment {
        name: name.to_string(),
        pct,
        pct_label: format!("{pct:.1}"),
        detail,
        slot,
        style: format!("flex-basis:{pct:.4}%"),
    }
}

fn bars(items: &[RepoCount]) -> Vec<Bar> {
    let max = items.iter().map(|i| i.count).max().unwrap_or(0);
    if max == 0 {
        return Vec::new();
    }

    items
        .iter()
        .map(|item| Bar {
            label: item.repo_name.clone(),
            value: format::count(item.count),
            style: format!("width:{:.4}%", item.count as f64 / max as f64 * 100.0),
        })
        .collect()
}

fn positive_duration(d: TimeDelta) -> Option<String> {
    (d > TimeDelta::zero()).then(|| format::duration(d, true))
}

fn human_date(date: Option<NaiveDate>) -> String {
    match date {
        Some(d) => format!("{} {}, {}", d.format("%b"), d.day(), d.year()),
        None => "\u{2014}".to_string(),
    }
}

fn safe_url(url: &str) -> String {
    let scheme_end = url.find([':', '/', '?', '#']);
    match scheme_end {
        Some(i) if url[i..].starts_with(':') => {
            let scheme = url[..i].to_ascii_lowercase();
            if matches!(scheme.as_str(), "http" | "https" | "mailto") {
                url.to_string()
            } else {
                "#".to_string()
            }
        }
        _ => url.to_string(),
    }
}

fn external_url(url: &str) -> String {
    let url = url.trim();
    if url.is_empty() {
        return String::new();
    }
    if url.starts_with("//") {
        return format!("https:{url}");
    }
    let has_scheme = url
        .find([':', '/', '?', '#'])
        .is_some_and(|i| url[i..].starts_with(':') && !url[..i].contains('.'));
    if has_scheme {
        safe_url(url)
    } else {
        format!("https://{url}")
    }
}

fn script_safe_json(value: &impl Serialize) -> Result<String> {
    let json = serde_json::to_string(value).context("failed to encode chart data")?;
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{ContributionDay, ContributionYear};

    fn sample_stats() -> UserStats {
        UserStats {
            username: "octocat".into(),
            blog: "javascript:alert(1)".into(),
            bio: "<script>alert(1)</script>".into(),
            contributions: vec![ContributionDay {
                date: "2026-01-05".parse().unwrap(),
                count: 3,
            }],
            contribution_years: vec![ContributionYear {
                year: 2026,
                total: 3,
                commits: 3,
                ..ContributionYear::default()
            }],
            pr_stats: Some(PullRequestStats {
                total: 4,
                merged: 2,
                open: 1,
                closed: 1,
                ..PullRequestStats::default()
            }),
            ..UserStats::default()
        }
    }

    #[tokio::test]
    async fn server_serves_page_and_shuts_down() {
        let server = Server::new(&sample_stats(), "127.0.0.1", 0).await.unwrap();
        let url = server.url();
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(server.run(async {
            let _ = stop_rx.await;
        }));

        let page = reqwest::get(&url).await.unwrap();
        assert_eq!(page.status(), reqwest::StatusCode::OK);
        assert!(
            page.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        assert!(page.text().await.unwrap().contains("@octocat"));

        let missing = reqwest::get(format!("{url}/nope")).await.unwrap();
        assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);

        let rebound = reqwest::Client::new()
            .get(&url)
            .header("host", "evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(rebound.status(), reqwest::StatusCode::FORBIDDEN);

        stop_tx.send(()).unwrap();
        handle.await.unwrap().unwrap();
    }

    #[test]
    fn render_embeds_chart_data() {
        let html = render(&sample_stats(), Local::now()).unwrap();
        for want in [
            r#"id="chart-data""#,
            r#""date":"2026-01-05""#,
            r#"data-chart="heatmap""#,
            r#"data-chart="mix""#,
            "Closed unmerged",
        ] {
            assert!(html.contains(want), "rendered page is missing {want:?}");
        }
    }

    #[test]
    fn render_escapes_user_content() {
        let html = render(&sample_stats(), Local::now()).unwrap();
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(!html.contains(r#"href="javascript:"#));
    }

    #[test]
    fn local_hosts_only() {
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        for ok in [
            "127.0.0.1:8080",
            "localhost:8080",
            "LOCALHOST:8080",
            "[::1]:8080",
        ] {
            assert!(is_local_host(ok, addr), "{ok}");
        }
        for bad in [
            "evil.example:8080",
            "127.0.0.1:9090",
            "localhost",
            "127.0.0.1:x",
            "[::1]",
            "",
        ] {
            assert!(!is_local_host(bad, addr), "{bad}");
        }
        let port80: SocketAddr = "127.0.0.1:80".parse().unwrap();
        assert!(is_local_host("localhost", port80));
    }

    #[test]
    fn safe_url_filters_schemes() {
        assert_eq!(safe_url("https://example.com"), "https://example.com");
        assert_eq!(safe_url("example.com/a:b"), "example.com/a:b");
        assert_eq!(safe_url("JavaScript:alert(1)"), "#");
        assert_eq!(safe_url("data:text/html,x"), "#");
    }

    #[test]
    fn external_url_adds_scheme() {
        assert_eq!(external_url("example.com"), "https://example.com");
        assert_eq!(
            external_url(" example.com/blog "),
            "https://example.com/blog"
        );
        assert_eq!(
            external_url("example.com:8080/x"),
            "https://example.com:8080/x"
        );
        assert_eq!(external_url("//example.com"), "https://example.com");
        assert_eq!(external_url("http://example.com"), "http://example.com");
        assert_eq!(external_url("javascript:alert(1)"), "#");
        assert_eq!(external_url(""), "");
    }

    #[test]
    fn render_links_blog_without_scheme() {
        let stats = UserStats {
            username: "octocat".into(),
            blog: "example.com".into(),
            ..UserStats::default()
        };
        let html = render(&stats, Local::now()).unwrap();
        assert!(
            html.contains(r#"href="https:&#x2f;&#x2f;example.com""#),
            "{html}"
        );
    }

    #[test]
    fn script_json_cannot_close_the_element() {
        let json = script_safe_json(&serde_json::json!({ "s": "</script>" })).unwrap();
        assert_eq!(json, r#"{"s":"\u003c/script\u003e"}"#);
    }

    #[test]
    fn bars_scale_to_largest() {
        let items = [
            RepoCount {
                repo_name: "a".into(),
                count: 4,
            },
            RepoCount {
                repo_name: "b".into(),
                count: 1,
            },
        ];
        let got = bars(&items);
        assert_eq!(got[0].style, "width:100.0000%");
        assert_eq!(got[1].style, "width:25.0000%");
    }

    #[test]
    fn other_language_slot() {
        let mut stats = UserStats::default();
        for (i, name) in ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
            .iter()
            .enumerate()
        {
            stats.languages.insert(name.to_string(), 100 - i as u64);
        }
        let segments = language_segments(&stats);
        assert_eq!(segments.len(), MAX_LANGUAGE_SLOTS + 1);
        assert_eq!(segments.last().unwrap().name, "Other");
        assert_eq!(segments.last().unwrap().slot, 0);
    }
}

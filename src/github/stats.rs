use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Days, NaiveDate, Timelike, Utc, Weekday};

use super::client::{ApiRepo, ApiUser, Client};
use super::types::{AccountAge, RepoYear, Repository, UserStats};

/// Gathers a user's statistics. Problems that only make the result less
/// complete are reported as warnings instead of errors.
pub struct StatsCalculator<'a> {
    client: &'a Client,
    warnings: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct StreakInfo {
    pub current: u32,
    pub max: u32,
    pub current_start: Option<NaiveDate>,
    pub max_start: Option<NaiveDate>,
    pub max_end: Option<NaiveDate>,
    pub active_days: u32,
}

impl<'a> StatsCalculator<'a> {
    pub fn new(client: &'a Client) -> Self {
        Self {
            client,
            warnings: Vec::new(),
        }
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub async fn calculate(&mut self, username: &str, full_scan: bool) -> Result<UserStats> {
        let mut stats = UserStats {
            username: username.to_string(),
            ..UserStats::default()
        };

        let user = self.client.user(username).await?;
        populate_profile(&mut stats, user, Utc::now());

        let repos = self
            .client
            .repositories(username)
            .await
            .context("failed to get repositories")?;

        for repo in &repos {
            stats.total_stars += repo.stargazers_count;
            stats.total_forks += repo.forks_count;
        }

        let (languages, err) = self.client.languages(&repos).await;
        if let Some(err) = err {
            self.warnings
                .push(format!("failed to get complete language stats: {err:#}"));
        }
        stats.languages = languages;
        stats.repos_by_year = repos_by_year(&repos);

        let mut commit_dates: Vec<DateTime<Utc>> = Vec::new();
        match self
            .client
            .contribution_calendar(username, stats.created_at)
            .await
        {
            Ok(calendar) => {
                commit_dates = calendar
                    .days
                    .iter()
                    .map(|d| d.date.and_time(Default::default()).and_utc())
                    .collect();
                stats.contributions = calendar.days;
                stats.contribution_years = calendar.years;
            }
            Err(err) => self.warnings.push(format!(
                "contribution calendar unavailable, falling back to commit history: {err:#}"
            )),
        }

        if commit_dates.is_empty() {
            commit_dates = self
                .client
                .commit_activity(username, full_scan)
                .await
                .context("failed to get commit activity")?;
        }

        let days: Vec<NaiveDate> = commit_dates.iter().map(DateTime::date_naive).collect();
        let streaks = calculate_streaks(&days, Utc::now().date_naive());
        stats.current_streak = streaks.current;
        stats.max_streak = streaks.max;
        stats.current_streak_start = streaks.current_start;
        stats.max_streak_start = streaks.max_start;
        stats.max_streak_end = streaks.max_end;
        stats.total_commit_days = streaks.active_days;

        (stats.most_active_day, stats.most_active_hour) = activity_patterns(&commit_dates);
        stats.top_repositories = top_repositories(&repos, 5);

        let (top_by_commits, prs, issues, reviews) = tokio::join!(
            self.client.top_repos_by_commits(username, &repos, 5),
            self.client.pull_requests(username),
            self.client.issues(username),
            self.client.reviews(username),
        );

        stats.top_repos_by_commits = top_by_commits;
        match prs {
            Ok(prs) => stats.pr_stats = Some(prs),
            Err(err) => self
                .warnings
                .push(format!("failed to get PR stats: {err:#}")),
        }
        match issues {
            Ok(issues) => stats.issue_stats = Some(issues),
            Err(err) => self
                .warnings
                .push(format!("failed to get issue stats: {err:#}")),
        }
        match reviews {
            Ok(reviews) => stats.review_stats = Some(reviews),
            Err(err) => self
                .warnings
                .push(format!("failed to get review stats: {err:#}")),
        }

        Ok(stats)
    }
}

fn populate_profile(stats: &mut UserStats, user: ApiUser, now: DateTime<Utc>) {
    stats.name = user.name.unwrap_or_default();
    stats.avatar_url = user.avatar_url.unwrap_or_default();
    stats.bio = user.bio.unwrap_or_default();
    stats.company = user.company.unwrap_or_default();
    stats.location = user.location.unwrap_or_default();
    stats.email = user.email.unwrap_or_default();
    stats.blog = user.blog.unwrap_or_default();
    stats.created_at = user.created_at;
    stats.updated_at = user.updated_at;
    stats.public_repos = user.public_repos;
    stats.public_gists = user.public_gists;
    stats.followers = user.followers;
    stats.following = user.following;
    if let Some(created) = user.created_at {
        stats.account_age = account_age(created, now);
    }
}

/// Finds the longest run of consecutive active days and the run that is still
/// going (one that ended today or yesterday).
pub fn calculate_streaks(dates: &[NaiveDate], today: NaiveDate) -> StreakInfo {
    let unique: Vec<NaiveDate> = dates
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let (Some(&first), Some(&last)) = (unique.first(), unique.last()) else {
        return StreakInfo::default();
    };

    let mut current = 1;
    let mut current_start = first;
    let mut max = 1;
    let mut max_start = first;
    let mut max_end = first;

    for pair in unique.windows(2) {
        if (pair[1] - pair[0]).num_days() == 1 {
            current += 1;
        } else {
            if current > max {
                max = current;
                max_start = current_start;
                max_end = pair[0];
            }
            current = 1;
            current_start = pair[1];
        }
    }

    if current > max {
        max = current;
        max_start = current_start;
        max_end = last;
    }

    let still_going = (today - last).num_days() <= 1;

    StreakInfo {
        current: if still_going { current } else { 0 },
        max,
        current_start: still_going.then_some(current_start),
        max_start: Some(max_start),
        max_end: Some(max_end),
        active_days: unique.len() as u32,
    }
}

/// Returns the weekday with the most activity and, when the timestamps carry
/// a time of day, the busiest hour. Ties go to the earlier day or hour.
pub fn activity_patterns(dates: &[DateTime<Utc>]) -> (Option<String>, Option<u32>) {
    if dates.is_empty() {
        return (None, None);
    }

    let mut by_day = [0u32; 7];
    let mut by_hour = [0u32; 24];
    let mut has_time_of_day = false;

    for date in dates {
        by_day[date.weekday().num_days_from_sunday() as usize] += 1;
        by_hour[date.hour() as usize] += 1;
        has_time_of_day |= date.num_seconds_from_midnight() != 0;
    }

    const SUNDAY_FIRST: [Weekday; 7] = [
        Weekday::Sun,
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
    ];
    let day = weekday_name(SUNDAY_FIRST[busiest(&by_day).unwrap_or(0)]);
    let hour = if has_time_of_day {
        busiest(&by_hour).map(|h| h as u32)
    } else {
        None
    };

    (Some(day.to_string()), hour)
}

/// Index of the largest non-zero count, preferring the first on ties.
fn busiest(counts: &[u32]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (i, &count) in counts.iter().enumerate() {
        if count > best.map_or(0, |b| counts[b]) {
            best = Some(i);
        }
    }
    best
}

pub fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// Non-fork repositories with the most stars.
fn top_repositories(repos: &[ApiRepo], limit: usize) -> Vec<Repository> {
    let mut list: Vec<Repository> = repos
        .iter()
        .filter(|r| !r.fork)
        .map(|r| Repository {
            name: r.name.clone(),
            description: r.description.clone().unwrap_or_default(),
            stars: r.stargazers_count,
            forks: r.forks_count,
            language: r.language.clone().unwrap_or_default(),
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect();
    list.sort_by_key(|r| std::cmp::Reverse(r.stars));
    list.truncate(limit);
    list
}

/// Counts repositories created each year. Years without new repositories in
/// between are included so charts get an honest time axis.
fn repos_by_year(repos: &[ApiRepo]) -> Vec<RepoYear> {
    let mut by_year: BTreeMap<i32, RepoYear> = BTreeMap::new();
    for repo in repos {
        let Some(created) = repo.created_at else {
            continue;
        };
        let year = created.year();
        let entry = by_year.entry(year).or_insert(RepoYear {
            year,
            ..RepoYear::default()
        });
        if repo.fork {
            entry.forks += 1;
        } else {
            entry.sources += 1;
        }
    }

    let (Some(&first), Some(&last)) = (by_year.keys().next(), by_year.keys().next_back()) else {
        return Vec::new();
    };

    (first..=last)
        .map(|year| {
            by_year.get(&year).copied().unwrap_or(RepoYear {
                year,
                ..RepoYear::default()
            })
        })
        .collect()
}

/// Shifts a timestamp by whole years and months with Go's `AddDate`
/// semantics: a day that does not exist in the target month overflows into
/// the next one (Jan 31 + 1 month = Mar 3 or Mar 2).
fn add_months(t: DateTime<Utc>, months: i32) -> DateTime<Utc> {
    let total = t.year() * 12 + t.month0() as i32 + months;
    let first = NaiveDate::from_ymd_opt(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1)
        .expect("valid first day of month");
    let date = first + Days::new(u64::from(t.day() - 1));
    date.and_time(t.time()).and_utc()
}

/// Calendar distance between two instants in whole years, months and days.
pub fn account_age(start: DateTime<Utc>, end: DateTime<Utc>) -> AccountAge {
    let mut years = 0;
    while add_months(start, (years + 1) * 12) <= end {
        years += 1;
    }
    let start = add_months(start, years * 12);

    let mut months = 0;
    while add_months(start, months + 1) <= end {
        months += 1;
    }
    let start = add_months(start, months);

    AccountAge {
        years: years as u32,
        months: months as u32,
        days: (end - start).num_days().max(0) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn streaks_with_current_run() {
        let dates = [
            d("2026-01-01"),
            d("2026-01-02"),
            d("2026-01-03"),
            d("2026-01-10"),
            d("2026-01-11"),
            d("2026-01-11"),
        ];
        let info = calculate_streaks(&dates, d("2026-01-12"));
        assert_eq!(
            info,
            StreakInfo {
                current: 2,
                max: 3,
                current_start: Some(d("2026-01-10")),
                max_start: Some(d("2026-01-01")),
                max_end: Some(d("2026-01-03")),
                active_days: 5,
            }
        );
    }

    #[test]
    fn streaks_inactive_and_trailing_max() {
        let dates = [d("2026-01-01"), d("2026-01-05"), d("2026-01-06")];
        let info = calculate_streaks(&dates, d("2026-02-01"));
        assert_eq!(info.current, 0);
        assert_eq!(info.current_start, None);
        assert_eq!(info.max, 2);
        assert_eq!(info.max_start, Some(d("2026-01-05")));
        assert_eq!(info.max_end, Some(d("2026-01-06")));
        assert_eq!(
            calculate_streaks(&[], d("2026-01-01")),
            StreakInfo::default()
        );
    }

    #[test]
    fn activity_patterns_without_time_of_day() {
        // 2026-01-05 and 2026-01-12 are Mondays.
        let dates: Vec<_> = ["2026-01-05", "2026-01-12", "2026-01-07"]
            .iter()
            .map(|s| d(s).and_time(Default::default()).and_utc())
            .collect();
        assert_eq!(
            activity_patterns(&dates),
            (Some("Monday".to_string()), None)
        );
    }

    #[test]
    fn activity_patterns_with_hours() {
        let dates = [
            Utc.with_ymd_and_hms(2026, 1, 4, 14, 30, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 1, 5, 14, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 1, 6, 9, 0, 0).unwrap(),
        ];
        // Each weekday occurs once, so the tie goes to Sunday.
        assert_eq!(
            activity_patterns(&dates),
            (Some("Sunday".to_string()), Some(14))
        );
    }

    #[test]
    fn account_age_like_go() {
        let t = |y, m, d| Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap();
        assert_eq!(
            account_age(t(2020, 3, 15), t(2026, 5, 20)),
            AccountAge {
                years: 6,
                months: 2,
                days: 5
            }
        );
        // Feb 31 overflows past Mar 2, so no whole month has passed.
        assert_eq!(
            account_age(t(2026, 1, 31), t(2026, 3, 2)),
            AccountAge {
                years: 0,
                months: 0,
                days: 30
            }
        );
    }

    #[test]
    fn go_style_month_overflow() {
        let t = Utc.with_ymd_and_hms(2026, 1, 31, 0, 0, 0).unwrap();
        assert_eq!(add_months(t, 1).date_naive(), d("2026-03-03"));
        assert_eq!(add_months(t, -2).date_naive(), d("2025-12-01"));
    }
}

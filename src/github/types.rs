use std::collections::{BTreeMap, HashMap};
use std::fmt;

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use serde::Serialize;

use crate::format::units;

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct UserStats {
    pub username: String,
    pub name: String,
    #[serde(rename = "AvatarURL")]
    pub avatar_url: String,
    pub bio: String,
    pub company: String,
    pub location: String,
    pub email: String,
    pub blog: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub public_repos: u64,
    pub public_gists: u64,
    pub followers: u64,
    pub following: u64,
    pub includes_private_repos: bool,
    pub total_repos: u64,

    pub account_age: AccountAge,
    pub total_stars: u64,
    pub total_forks: u64,
    pub current_streak: u32,
    pub max_streak: u32,
    #[serde(with = "serde_date::opt")]
    pub current_streak_start: Option<NaiveDate>,
    #[serde(with = "serde_date::opt")]
    pub max_streak_start: Option<NaiveDate>,
    #[serde(with = "serde_date::opt")]
    pub max_streak_end: Option<NaiveDate>,
    #[serde(rename = "TotalCommitDays")]
    pub total_active_days: u32,

    pub languages: BTreeMap<String, u64>,
    pub most_active_day: Option<String>,
    #[serde(serialize_with = "serialize_hour")]
    pub most_active_hour: Option<u32>,
    pub top_repositories: Vec<Repository>,
    pub top_repos_by_commits: Vec<RepoCount>,

    pub contributions: Vec<ContributionDay>,
    pub contribution_years: Vec<ContributionYear>,
    pub repos_by_year: Vec<RepoYear>,

    #[serde(rename = "PRStats")]
    pub pr_stats: Option<PullRequestStats>,
    pub issue_stats: Option<IssueStats>,
    pub review_stats: Option<ReviewStats>,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Repository {
    pub name: String,
    pub description: String,
    pub stars: u64,
    pub forks: u64,
    pub language: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AccountAge {
    pub years: u32,
    pub months: u32,
    pub days: u32,
}

impl fmt::Display for AccountAge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let unit = |n: u32, word| units(i64::from(n), word);
        match (self.years, self.months, self.days) {
            (0, 0, d) => f.write_str(&unit(d, "day")),
            (0, m, 0) => f.write_str(&unit(m, "month")),
            (0, m, d) => write!(f, "{}, {}", unit(m, "month"), unit(d, "day")),
            (y, 0, _) => f.write_str(&unit(y, "year")),
            (y, m, _) => write!(f, "{}, {}", unit(y, "year"), unit(m, "month")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContributionDay {
    #[serde(with = "serde_date")]
    pub date: NaiveDate,
    pub count: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ContributionYear {
    pub year: i32,
    pub total: u32,
    pub commits: u32,
    pub pull_requests: u32,
    pub issues: u32,
    pub reviews: u32,
    pub repositories: u32,
    pub private: u32,
}

#[derive(Debug, Default)]
pub struct ContributionCalendar {
    pub days: Vec<ContributionDay>,
    pub years: Vec<ContributionYear>,
    pub reviews_by_repo: HashMap<String, u64>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RepoYear {
    pub year: i32,
    pub sources: u32,
    pub forks: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RepoCount {
    pub repo_name: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct OpenPullRequest {
    pub title: String,
    pub repo_name: String,
    pub number: u64,
    pub created_at: Option<DateTime<Utc>>,
    #[serde(rename = "URL")]
    pub url: String,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PullRequestStats {
    pub total: u64,
    pub open: u64,
    pub closed: u64,
    pub merged: u64,
    #[serde(with = "serde_nanos")]
    pub avg_merge_time: TimeDelta,
    pub top_repos: Vec<RepoCount>,
    #[serde(rename = "OpenPRs")]
    pub open_prs: Vec<OpenPullRequest>,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct IssueStats {
    pub total: u64,
    pub open: u64,
    pub closed: u64,
    #[serde(with = "serde_nanos")]
    pub avg_close_time: TimeDelta,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReviewStats {
    pub total: u64,
    pub top_repos: Vec<RepoCount>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LanguageStat {
    pub name: String,
    pub bytes: u64,
    pub percentage: f64,
}

pub fn language_stats(languages: &BTreeMap<String, u64>) -> Vec<LanguageStat> {
    let total: u64 = languages.values().sum();
    let mut stats: Vec<LanguageStat> = languages
        .iter()
        .map(|(name, &bytes)| LanguageStat {
            name: name.clone(),
            bytes,
            percentage: if total > 0 {
                bytes as f64 / total as f64 * 100.0
            } else {
                0.0
            },
        })
        .collect();
    stats.sort_by_key(|l| std::cmp::Reverse(l.bytes));
    stats
}

fn serialize_hour<S: serde::Serializer>(hour: &Option<u32>, s: S) -> Result<S::Ok, S::Error> {
    match hour {
        Some(h) => s.serialize_i64(i64::from(*h)),
        None => s.serialize_i64(-1),
    }
}

mod serde_date {
    use chrono::NaiveDate;
    use serde::Serializer;

    pub fn serialize<S: Serializer>(date: &NaiveDate, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&date.format("%Y-%m-%dT00:00:00Z"))
    }

    pub mod opt {
        use chrono::NaiveDate;
        use serde::Serializer;

        pub fn serialize<S: Serializer>(date: &Option<NaiveDate>, s: S) -> Result<S::Ok, S::Error> {
            match date {
                Some(date) => super::serialize(date, s),
                None => s.serialize_none(),
            }
        }
    }
}

mod serde_nanos {
    use chrono::TimeDelta;
    use serde::Serializer;

    pub fn serialize<S: Serializer>(d: &TimeDelta, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(d.num_nanoseconds().unwrap_or(i64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_age_display() {
        let age = |years, months, days| {
            AccountAge {
                years,
                months,
                days,
            }
            .to_string()
        };
        assert_eq!(age(3, 2, 5), "3 years, 2 months");
        assert_eq!(age(3, 0, 5), "3 years");
        assert_eq!(age(0, 4, 1), "4 months, 1 day");
        assert_eq!(age(1, 1, 0), "1 year, 1 month");
        assert_eq!(age(0, 4, 0), "4 months");
        assert_eq!(age(0, 0, 9), "9 days");
    }

    #[test]
    fn json_keys_match_go_version() {
        let stats = UserStats {
            current_streak_start: Some("2026-01-05".parse().unwrap()),
            pr_stats: Some(PullRequestStats {
                avg_merge_time: TimeDelta::seconds(2),
                ..PullRequestStats::default()
            }),
            ..UserStats::default()
        };
        let json = serde_json::to_value(&stats).unwrap();
        assert_eq!(json["MostActiveHour"], -1);
        assert_eq!(json["CurrentStreakStart"], "2026-01-05T00:00:00Z");
        assert_eq!(json["PRStats"]["AvgMergeTime"], 2_000_000_000);
        assert!(json["PRStats"]["OpenPRs"].is_array());
        assert!(json.get("AvatarURL").is_some());
        assert!(json["AccountAge"].get("Years").is_some());
        assert!(json.get("TotalCommitDays").is_some());
    }

    #[test]
    fn language_stats_sorted_by_size() {
        let langs = BTreeMap::from([("Go".to_string(), 100), ("Rust".to_string(), 300)]);
        let stats = language_stats(&langs);
        assert_eq!(stats[0].name, "Rust");
        assert_eq!(stats[0].percentage, 75.0);
        assert_eq!(stats[1].percentage, 25.0);
    }
}

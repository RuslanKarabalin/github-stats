//! Data for the interactive charts. It is embedded in the page as JSON and
//! drawn by charts.js.

use std::collections::HashMap;

use chrono::{Datelike, Months, NaiveDate, Weekday};
use serde::Serialize;

use crate::github::{ContributionDay, UserStats, weekday_name};

#[derive(Debug, Default, Serialize)]
pub struct ChartData {
    pub today: String,
    pub days: Vec<DayPoint>,
    pub monthly: Vec<MonthPoint>,
    pub weekdays: Vec<WeekdayPoint>,
    pub years: Vec<YearPoint>,
    pub repos: Vec<RepoPoint>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct DayPoint {
    pub date: String,
    pub count: u32,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct MonthPoint {
    pub month: String,
    pub count: u32,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct WeekdayPoint {
    pub name: &'static str,
    pub total: u32,
    pub average: f64,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct YearPoint {
    pub year: i32,
    pub total: u32,
    pub commits: u32,
    pub pull_requests: u32,
    pub issues: u32,
    pub reviews: u32,
    pub other: u32,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct RepoPoint {
    pub year: i32,
    pub sources: u32,
    pub forks: u32,
}

#[derive(Debug, PartialEq)]
pub struct ContributionSummary {
    pub total: u32,
    pub this_year: u32,
    pub best_count: u32,
    pub best_date: NaiveDate,
    pub active_avg: f64,
}

pub fn build_chart_data(stats: &UserStats, today: NaiveDate) -> ChartData {
    ChartData {
        today: today.format("%Y-%m-%d").to_string(),
        days: stats
            .contributions
            .iter()
            .map(|d| DayPoint {
                date: d.date.format("%Y-%m-%d").to_string(),
                count: d.count,
            })
            .collect(),
        monthly: monthly_contributions(&stats.contributions, today),
        weekdays: weekday_contributions(&stats.contributions, today),
        years: stats
            .contribution_years
            .iter()
            .map(|y| {
                let known = y.commits + y.pull_requests + y.issues + y.reviews;
                YearPoint {
                    year: y.year,
                    total: y.total.max(known),
                    commits: y.commits,
                    pull_requests: y.pull_requests,
                    issues: y.issues,
                    reviews: y.reviews,
                    other: y.total.saturating_sub(known),
                }
            })
            .collect(),
        repos: stats
            .repos_by_year
            .iter()
            .map(|r| RepoPoint {
                year: r.year,
                sources: r.sources,
                forks: r.forks,
            })
            .collect(),
    }
}

pub fn summarize_contributions(
    days: &[ContributionDay],
    today: NaiveDate,
) -> Option<ContributionSummary> {
    let first = days.first()?;
    let mut summary = ContributionSummary {
        total: 0,
        this_year: 0,
        best_count: 0,
        best_date: first.date,
        active_avg: 0.0,
    };

    for day in days {
        summary.total += day.count;
        if day.date.year() == today.year() {
            summary.this_year += day.count;
        }
        if day.count > summary.best_count {
            summary.best_count = day.count;
            summary.best_date = day.date;
        }
    }
    summary.active_avg = f64::from(summary.total) / days.len() as f64;

    Some(summary)
}

/// Totals each month from the first active month to `today`, including
/// months with no contributions.
pub fn monthly_contributions(days: &[ContributionDay], today: NaiveDate) -> Vec<MonthPoint> {
    let Some(first) = days.first() else {
        return Vec::new();
    };

    let mut totals: HashMap<(i32, u32), u32> = HashMap::new();
    for day in days {
        *totals
            .entry((day.date.year(), day.date.month()))
            .or_default() += day.count;
    }

    let last = today.with_day(1).expect("first of month");
    let mut month = first.date.with_day(1).expect("first of month");
    let mut points = Vec::new();
    while month <= last {
        points.push(MonthPoint {
            month: month.format("%Y-%m").to_string(),
            count: totals
                .get(&(month.year(), month.month()))
                .copied()
                .unwrap_or(0),
        });
        month = month + Months::new(1);
    }

    points
}

/// Averages contributions per weekday over every day from the first active
/// day to `today`, so idle days count too. Weeks start on Monday.
pub fn weekday_contributions(days: &[ContributionDay], today: NaiveDate) -> Vec<WeekdayPoint> {
    let Some(first) = days.first() else {
        return Vec::new();
    };

    let mut totals = [0u32; 7];
    for day in days {
        totals[day.date.weekday().num_days_from_monday() as usize] += day.count;
    }

    let mut occurrences = [0u32; 7];
    for date in first.date.iter_days().take_while(|d| *d <= today) {
        occurrences[date.weekday().num_days_from_monday() as usize] += 1;
    }

    std::iter::successors(Some(Weekday::Mon), |d| Some(d.succ()))
        .zip(totals.into_iter().zip(occurrences))
        .map(|(weekday, (total, seen))| WeekdayPoint {
            name: &weekday_name(weekday)[..3],
            total,
            average: if seen > 0 {
                f64::from(total) / f64::from(seen)
            } else {
                0.0
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::ContributionYear;

    fn day(s: &str, count: u32) -> ContributionDay {
        ContributionDay {
            date: s.parse().unwrap(),
            count,
        }
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn monthly_contributions_fill_gaps() {
        let days = [
            day("2026-01-05", 3),
            day("2026-01-20", 2),
            day("2026-03-01", 4),
        ];
        let got = monthly_contributions(&days, date("2026-04-10"));
        let want: Vec<_> = [
            ("2026-01", 5),
            ("2026-02", 0),
            ("2026-03", 4),
            ("2026-04", 0),
        ]
        .into_iter()
        .map(|(m, c)| MonthPoint {
            month: m.to_string(),
            count: c,
        })
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn weekday_contributions_average_over_idle_days() {
        // 2026-01-05 is a Monday; the range covers two Mondays.
        let got = weekday_contributions(&[day("2026-01-05", 4)], date("2026-01-12"));
        assert_eq!(got.len(), 7);
        assert_eq!((got[0].name, got[0].total, got[0].average), ("Mon", 4, 2.0));
        assert_eq!((got[6].name, got[6].total), ("Sun", 0));
    }

    #[test]
    fn summarizes_contributions() {
        let days = [
            day("2025-12-31", 6),
            day("2026-02-01", 9),
            day("2026-03-01", 3),
        ];
        let got = summarize_contributions(&days, date("2026-06-01")).unwrap();
        assert_eq!(
            got,
            ContributionSummary {
                total: 18,
                this_year: 12,
                best_count: 9,
                best_date: date("2026-02-01"),
                active_avg: 6.0,
            }
        );
        assert_eq!(summarize_contributions(&[], date("2026-06-01")), None);
    }

    #[test]
    fn folds_unknown_contribution_types_into_other() {
        let stats = UserStats {
            contribution_years: vec![ContributionYear {
                year: 2026,
                total: 20,
                commits: 10,
                pull_requests: 3,
                issues: 2,
                reviews: 1,
                ..ContributionYear::default()
            }],
            ..UserStats::default()
        };
        let got = build_chart_data(&stats, date("2026-06-01"));
        assert_eq!((got.years[0].other, got.years[0].total), (4, 20));
    }
}

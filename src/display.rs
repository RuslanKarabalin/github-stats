//! Terminal output: status messages and the statistics report.

use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use comfy_table::{
    Cell, CellAlignment, ContentArrangement, ContentLineStyle, LineStyle, Table, TableStyle,
};
use console::style;

use crate::config::Format;
use crate::format;
use crate::github::{UserStats, language_stats};

const WIDTH: usize = 80;
/// Light box drawing without lines between body rows.
const TABLE_STYLE: TableStyle = TableStyle::new()
    .top_border(LineStyle::new('┌', '─', '┬', '┐'))
    .header_lines(ContentLineStyle::new('│', '│', '│'))
    .header_separator(LineStyle::new('├', '─', '┼', '┤'))
    .content_lines(ContentLineStyle::new('│', '│', '│'))
    .bottom_border(LineStyle::new('└', '─', '┴', '┘'));

pub fn success(message: &str) {
    println!("{}", style(format!("✓ {message}")).green());
}

pub fn warning(message: &str) {
    println!("{}", style(format!("⚠ {message}")).yellow());
}

pub fn error(message: &str) {
    println!("{}", style(format!("✗ {message}")).red().bold());
}

pub fn heading(message: &str) {
    println!("{}", style(message).cyan().bold());
}

pub fn print_stats(stats: &UserStats, format: Format) -> Result<()> {
    match format {
        Format::Json => {
            println!("{}", serde_json::to_string_pretty(stats)?);
            Ok(())
        }
        Format::Table => {
            print_tables(stats);
            Ok(())
        }
    }
}

fn table<const N: usize>(headers: [&str; N], rows: impl IntoIterator<Item = [String; N]>) -> Table {
    let mut table = Table::new();
    table
        .load_style(TABLE_STYLE)
        .set_content_arrangement(ContentArrangement::Disabled)
        .set_header(
            headers
                .iter()
                .map(|h| Cell::new(h.to_uppercase()).set_alignment(CellAlignment::Center)),
        );
    for row in rows {
        table.add_row(row);
    }
    table
}

fn section(title: &str) {
    println!();
    println!("{}", style(title).green());
    println!("{}", "-".repeat(WIDTH));
}

fn date_long(t: Option<DateTime<Utc>>) -> String {
    t.map(|t| t.format("%B %-d, %Y").to_string())
        .unwrap_or_default()
}

fn date_short<T: chrono::Datelike>(t: Option<T>) -> String {
    t.map(|t| format!("{} {}, {}", month_abbr(t.month()), t.day(), t.year()))
        .unwrap_or_default()
}

fn month_abbr(month: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    NAMES[(month as usize).saturating_sub(1) % 12]
}

fn print_tables(stats: &UserStats) {
    let rule = |c: &str| c.repeat(WIDTH);

    println!("{}", style(format!("\n{}", rule("="))).cyan().bold());
    println!(
        "{}",
        style(format!("  GitHub Statistics for @{}", stats.username))
            .cyan()
            .bold()
    );
    println!("{}", style(rule("=")).cyan().bold());

    section("👤 PROFILE");
    let mut rows = Vec::new();
    let mut push_if = |label: &str, value: &str| {
        if !value.is_empty() {
            rows.push([label.to_string(), value.to_string()]);
        }
    };
    push_if("Name", &stats.name);
    push_if("Username", &stats.username);
    push_if("Bio", &format::truncate(&stats.bio, 60));
    push_if("Company", &stats.company);
    push_if("Location", &stats.location);
    push_if("Website", &stats.blog);
    rows.extend([
        ["Joined".into(), date_long(stats.created_at)],
        ["Account Age".into(), stats.account_age.to_string()],
        ["Followers".into(), stats.followers.to_string()],
        ["Following".into(), stats.following.to_string()],
    ]);
    println!("{}", table(["Field", "Value"], rows));

    section("📚 REPOSITORY STATISTICS");
    println!(
        "{}",
        table(
            ["Metric", "Value"],
            [
                ["Public Repositories".into(), stats.public_repos.to_string()],
                ["Public Gists".into(), stats.public_gists.to_string()],
                [
                    "Total Stars Received".into(),
                    format!("{} ⭐", stats.total_stars)
                ],
                ["Total Forks Received".into(), stats.total_forks.to_string()],
            ],
        )
    );

    section("🔥 CONTRIBUTION STREAKS");
    let mut rows = Vec::new();
    if stats.current_streak > 0 {
        rows.push([
            "Current Streak".into(),
            format!("{} days 🔥", stats.current_streak),
        ]);
        rows.push([
            "Current Streak Start".into(),
            date_short(stats.current_streak_start),
        ]);
    } else {
        rows.push(["Current Streak".into(), "0 days (inactive)".into()]);
    }
    rows.push([
        "Maximum Streak".into(),
        format!("{} days 🏆", stats.max_streak),
    ]);
    if stats.max_streak_start.is_some() {
        rows.push([
            "Max Streak Period".into(),
            format!(
                "{} - {}",
                date_short(stats.max_streak_start),
                date_short(stats.max_streak_end)
            ),
        ]);
    }
    rows.push([
        "Total Active Days".into(),
        stats.total_commit_days.to_string(),
    ]);
    println!("{}", table(["Metric", "Value"], rows));

    if stats.most_active_day.is_some() || stats.most_active_hour.is_some() {
        section("📊 ACTIVITY PATTERNS");
        let mut rows = Vec::new();
        if let Some(day) = &stats.most_active_day {
            rows.push(["Most Active Day".into(), day.clone()]);
        }
        if let Some(hour) = stats.most_active_hour {
            rows.push(["Most Active Hour".into(), format::hour(hour)]);
        }
        println!("{}", table(["Metric", "Value"], rows));
    }

    if !stats.languages.is_empty() {
        section("💻 LANGUAGE STATISTICS");
        let rows = language_stats(&stats.languages)
            .into_iter()
            .take(10)
            .map(|l| {
                [
                    l.name,
                    format::bytes(l.bytes),
                    format!("{:.1}%", l.percentage),
                ]
            });
        println!("{}", table(["Language", "Bytes", "Percentage"], rows));
    }

    if !stats.top_repositories.is_empty() {
        section("🌟 TOP REPOSITORIES (by stars)");
        let rows = stats.top_repositories.iter().map(|r| {
            [
                r.name.clone(),
                format!("{} ⭐", r.stars),
                r.forks.to_string(),
                if r.language.is_empty() {
                    "N/A".into()
                } else {
                    r.language.clone()
                },
            ]
        });
        println!(
            "{}",
            table(["Repository", "Stars", "Forks", "Language"], rows)
        );
    }

    if !stats.top_repos_by_commits.is_empty() {
        section("📈 TOP REPOSITORIES (by commits)");
        let rows = stats
            .top_repos_by_commits
            .iter()
            .map(|r| [r.repo_name.clone(), r.count.to_string()]);
        println!("{}", table(["Repository", "Commits"], rows));
    }

    if let Some(prs) = stats.pr_stats.as_ref().filter(|p| p.total > 0) {
        section("🔀 PULL REQUEST STATISTICS");
        let mut rows = vec![
            ["Total PRs Created".into(), prs.total.to_string()],
            ["Open".into(), prs.open.to_string()],
            ["Merged".into(), format!("{} ✓", prs.merged)],
            ["Closed (unmerged)".into(), prs.closed.to_string()],
        ];
        if prs.avg_merge_time > chrono::TimeDelta::zero() {
            rows.push([
                "Avg Time to Merge".into(),
                format::duration(prs.avg_merge_time, "minutes"),
            ]);
        }
        println!("{}", table(["Metric", "Value"], rows));

        if !prs.top_repos.is_empty() {
            println!();
            println!("  Top Repositories by PR Count:");
            let rows = prs
                .top_repos
                .iter()
                .map(|r| [r.repo_name.clone(), r.count.to_string()]);
            println!("{}", table(["Repository", "PR"], rows));
        }

        if !prs.open_prs.is_empty() {
            println!();
            println!("  Currently Open Pull Requests:");
            let rows = prs.open_prs.iter().map(|pr| {
                [
                    pr.repo_name.clone(),
                    pr.number.to_string(),
                    format::truncate(&pr.title, 40),
                    date_short(pr.created_at),
                ]
            });
            println!("{}", table(["Repository", "#", "Title", "Created"], rows));
        }
    }

    if let Some(issues) = stats.issue_stats.as_ref().filter(|i| i.total > 0) {
        section("📋 ISSUE STATISTICS");
        let mut rows = vec![
            ["Total Issues Created".into(), issues.total.to_string()],
            ["Open".into(), issues.open.to_string()],
            ["Closed".into(), format!("{} ✓", issues.closed)],
        ];
        if issues.avg_close_time > chrono::TimeDelta::zero() {
            rows.push([
                "Avg Time to Close".into(),
                format::duration(issues.avg_close_time, "minutes"),
            ]);
        }
        println!("{}", table(["Metric", "Value"], rows));
    }

    if let Some(reviews) = stats.review_stats.as_ref().filter(|r| r.total > 0) {
        section("👀 CODE REVIEW STATISTICS");
        println!(
            "{}",
            table(
                ["Metric", "Value"],
                [["Total Reviews".into(), reviews.total.to_string()]]
            )
        );

        if !reviews.top_repos.is_empty() {
            println!();
            println!("  Top Repositories by Review Count:");
            let rows = reviews
                .top_repos
                .iter()
                .map(|r| [r.repo_name.clone(), r.count.to_string()]);
            println!("{}", table(["Repository", "Reviews"], rows));
        }
    }

    println!();
    println!("{}", style(rule("-")).blue());
    println!(
        "{}",
        style(format!(
            "Generated at: {}",
            Local::now().format("%Y-%m-%d %H:%M:%S %Z")
        ))
        .blue()
    );
    println!("{}", style(rule("=")).blue());
    println!();
}

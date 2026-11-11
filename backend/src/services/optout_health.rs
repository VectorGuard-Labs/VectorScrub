//! Is each broker's public opt-out page still reachable? See migration 0013.
//!
//! The agent layer fetches the pages nightly and reports here. Nothing in this module touches
//! customer data, and "reachable" is never used to claim that a removal works.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::OffsetDateTime;
use utoipa::ToSchema;

/// Most checks one report may carry. There are 19 brokers today; this leaves room to grow.
pub const MAX_CHECKS_PER_REPORT: usize = 200;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptOutPageStatus {
    /// The page loaded.
    Reachable,
    /// The page loaded, but its form carries a CAPTCHA: a person has to complete it.
    Captcha,
    /// The site refused our request (HTTP 401/403/451, or a "verify you are human" page).
    Blocked,
    /// The address no longer works (HTTP 404/410...): the link needs fixing.
    NotFound,
    /// Down, rate limited, redirected somewhere unexpected, or unreachable.
    Unavailable,
}

impl OptOutPageStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            OptOutPageStatus::Reachable => "reachable",
            OptOutPageStatus::Captcha => "captcha",
            OptOutPageStatus::Blocked => "blocked",
            OptOutPageStatus::NotFound => "not_found",
            OptOutPageStatus::Unavailable => "unavailable",
        }
    }

    /// Whether the page opened, i.e. the link is alive even if a person must finish the form.
    pub fn opened(self) -> bool {
        matches!(self, OptOutPageStatus::Reachable | OptOutPageStatus::Captcha)
    }
}

/// One page's result, as reported by the agent.
#[derive(Debug, Clone, Deserialize)]
pub struct OptOutPageCheck {
    pub source: String,
    pub status: OptOutPageStatus,
    pub http_status: Option<i32>,
}

impl OptOutPageCheck {
    /// A source is a hostname (letters, digits, dots, hyphens); the HTTP status, if any, is a real one.
    pub fn is_valid(&self) -> bool {
        !self.source.is_empty()
            && self.source.len() <= 100
            && self.source.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            && self.http_status.is_none_or(|s| (100..=599).contains(&s))
    }
}

/// What the public sees for one broker.
#[derive(Debug, Serialize, ToSchema)]
pub struct PublicOptOutHealth {
    pub source: String,
    pub status: OptOutPageStatus,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub checked_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_reachable_at: Option<OffsetDateTime>,
}

/// Store one report. Each source is overwritten, `last_reachable_at` survives a later failure,
/// and `consecutive_failures` counts failed checks in a row. One transaction: a report is
/// applied whole or not at all.
pub async fn record_checks(pool: &PgPool, checks: &[OptOutPageCheck]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for c in checks {
        sqlx::query(
            "INSERT INTO optout_health \
                (source, status, http_status, checked_at, last_reachable_at, consecutive_failures) \
             VALUES ($1, $2, $3, NOW(), CASE WHEN $4 THEN NOW() END, CASE WHEN $4 THEN 0 ELSE 1 END) \
             ON CONFLICT (source) DO UPDATE SET \
                status = EXCLUDED.status, \
                http_status = EXCLUDED.http_status, \
                checked_at = NOW(), \
                last_reachable_at = CASE WHEN $4 THEN NOW() ELSE optout_health.last_reachable_at END, \
                consecutive_failures = CASE WHEN $4 THEN 0 ELSE optout_health.consecutive_failures + 1 END",
        )
        .bind(&c.source)
        .bind(c.status.as_str())
        .bind(c.http_status)
        .bind(c.status.opened())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

pub async fn list_public(pool: &PgPool) -> Result<Vec<PublicOptOutHealth>, sqlx::Error> {
    let rows: Vec<(String, String, OffsetDateTime, Option<OffsetDateTime>)> = sqlx::query_as(
        "SELECT source, status, checked_at, last_reachable_at FROM optout_health ORDER BY source",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(source, status, checked_at, last_reachable_at)| {
            let status = serde_json::from_value(serde_json::Value::String(status)).ok()?;
            Some(PublicOptOutHealth { source, status, checked_at, last_reachable_at })
        })
        .collect())
}

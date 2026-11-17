//! Is this database, restored from a backup, one the service can actually run on?
//!
//! `vectorscrub verify-backup` runs these checks against a restored copy (see
//! infra/scripts/restore_verify.sh and docs/runbooks/backup-and-restore.md). A backup that
//! restores without errors can still be useless: the schema may be from another version, the
//! audit-log guards may be missing, or the encrypted data may not open with the master key you
//! hold. The last one is the one that matters most. Everything personal in this database is
//! encrypted, so a backup is only as good as the key that goes with it.
//!
//! The checks read only. The one that tries to delete an audit row does so inside a transaction
//! it always rolls back. Output is counts and names of problems, never a value from the data.

use anyhow::{Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

use crate::crypto::{self, MasterKey};

/// How many identity profiles and two-factor secrets are trial-decrypted by default.
pub const DEFAULT_SAMPLE: i64 = 50;

#[derive(Debug, Default)]
pub struct Report {
    pub customers: i64,
    pub migrations_expected: usize,
    pub migrations_applied: usize,
    pub identities_checked: i64,
    pub mfa_secrets_checked: i64,
    pub sealed_links_checked: i64,
    pub newest_audit_entry: Option<String>,
    /// Everything that is wrong, in words. Empty means the backup is usable.
    pub problems: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }

    /// Plain text for the operator. No customer data.
    pub fn render(&self) -> String {
        let mut out = String::new();
        let mut line = |s: String| {
            out.push_str(&s);
            out.push('\n');
        };
        line(format!("customers:                 {}", self.customers));
        line(format!("migrations:                {} applied, {} expected", self.migrations_applied, self.migrations_expected));
        line(format!("identity profiles opened:  {}", self.identities_checked));
        line(format!("two-factor secrets opened: {}", self.mfa_secrets_checked));
        line(format!("listing links opened:      {}", self.sealed_links_checked));
        line(format!("newest audit entry:        {}", self.newest_audit_entry.as_deref().unwrap_or("none")));
        if self.ok() {
            line("RESULT: usable".to_string());
        } else {
            for p in &self.problems {
                line(format!("PROBLEM: {p}"));
            }
            line("RESULT: NOT usable".to_string());
        }
        out
    }
}

/// Run every check. `sample` bounds how many encrypted rows of each kind are trial-decrypted.
pub async fn run(pool: &PgPool, master: &MasterKey, sample: i64) -> Result<Report> {
    let customers: i64 = sqlx::query_scalar("SELECT count(*) FROM customers").fetch_one(pool).await.context("count customers")?;
    let mut r = Report { customers, ..Report::default() };
    check_migrations(pool, &mut r).await?;
    check_audit_guards(pool, &mut r).await?;
    check_encrypted_data(pool, master, sample.max(1), &mut r).await?;

    r.newest_audit_entry = sqlx::query_scalar::<_, Option<time::OffsetDateTime>>("SELECT max(timestamp) FROM audit_log")
        .fetch_one(pool)
        .await
        .context("newest audit entry")?
        .map(|t| t.format(&time::format_description::well_known::Rfc3339).unwrap_or_default());
    Ok(r)
}

/// The restored schema must be exactly the one this build of the service expects.
async fn check_migrations(pool: &PgPool, r: &mut Report) -> Result<()> {
    let migrator = sqlx::migrate!("./migrations");
    let expected: Vec<(i64, Vec<u8>)> = migrator
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .map(|m| (m.version, m.checksum.to_vec()))
        .collect();
    r.migrations_expected = expected.len();

    let applied: Vec<(i64, Vec<u8>, bool)> =
        match sqlx::query_as("SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
        {
            Ok(rows) => rows,
            Err(_) => {
                r.problems.push("the migration history table is missing: this is not a VectorScrub database".into());
                return Ok(());
            }
        };
    r.migrations_applied = applied.iter().filter(|(_, _, ok)| *ok).count();

    for (version, checksum) in &expected {
        match applied.iter().find(|(v, _, _)| v == version) {
            None => r.problems.push(format!("migration {version} is not applied (the backup is from an older version; start the service to apply it)")),
            Some((_, _, false)) => r.problems.push(format!("migration {version} is recorded as failed")),
            Some((_, c, true)) if c != checksum => {
                r.problems.push(format!("migration {version} was applied from different SQL than this build contains"))
            }
            Some(_) => {}
        }
    }
    for (version, _, _) in &applied {
        if !expected.iter().any(|(v, _)| v == version) {
            r.problems.push(format!("migration {version} is unknown to this build (the backup is from a newer version)"));
        }
    }
    Ok(())
}

/// Both audit tables must still refuse edits. Triggers are not data, but they travel in the
/// dump, and a restore into a database where they were skipped would silently lose the guarantee.
async fn check_audit_guards(pool: &PgPool, r: &mut Report) -> Result<()> {
    for table in ["audit_log", "removal_audit_log"] {
        // Update/delete guard (row level) and truncate guard (statement level).
        let (row_guards, truncate_guards): (i64, i64) = sqlx::query_as(
            "SELECT count(*) FILTER (WHERE (tgtype & 1) = 1), count(*) FILTER (WHERE (tgtype & 32) = 32) \
             FROM pg_trigger WHERE tgrelid = to_regclass($1) AND NOT tgisinternal",
        )
        .bind(table)
        .fetch_one(pool)
        .await
        .with_context(|| format!("inspect triggers on {table}"))?;
        if row_guards == 0 || truncate_guards == 0 {
            r.problems.push(format!("{table} has lost its append-only triggers"));
            continue;
        }

        // And prove it behaves: try to delete one row, then roll back whatever happens.
        let mut tx = pool.begin().await?;
        let outcome = sqlx::query(&format!("DELETE FROM {table} WHERE ctid = (SELECT ctid FROM {table} LIMIT 1)"))
            .execute(&mut *tx)
            .await;
        tx.rollback().await.ok();
        match outcome {
            Err(e) if e.to_string().contains("append-only") => {}
            Err(e) => r.problems.push(format!("{table}: deleting a row failed for an unexpected reason: {}", first_line(&e.to_string()))),
            Ok(done) if done.rows_affected() == 0 => {} // empty table: the trigger check above stands
            Ok(_) => r.problems.push(format!("{table} allowed a row to be deleted")),
        }
    }
    Ok(())
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

/// Trial-decrypt a sample of each kind of encrypted value with the key you are holding.
async fn check_encrypted_data(pool: &PgPool, master: &MasterKey, sample: i64, r: &mut Report) -> Result<()> {
    let identities: Vec<(Uuid, Vec<u8>)> = sqlx::query_as("SELECT customer_id, full_name_enc FROM identity_profiles ORDER BY random() LIMIT $1")
        .bind(sample)
        .fetch_all(pool)
        .await
        .context("sample identity profiles")?;
    let mut failed = 0;
    for (customer, blob) in &identities {
        let key = crypto::derive_customer_key(master, *customer);
        if crypto::decrypt_str(&key, blob).is_err() {
            failed += 1;
        }
    }
    r.identities_checked = identities.len() as i64;
    if failed > 0 {
        r.problems.push(format!(
            "{failed} of {} sampled identity profiles do not open with this master key. This is either the wrong key or a damaged backup",
            identities.len()
        ));
    }

    let secrets: Vec<(Uuid, Vec<u8>)> = sqlx::query_as("SELECT customer_id, secret_enc FROM customer_mfa ORDER BY random() LIMIT $1")
        .bind(sample)
        .fetch_all(pool)
        .await
        .context("sample two-factor secrets")?;
    let failed = secrets
        .iter()
        .filter(|(c, blob)| crypto::decrypt(&crypto::derive_customer_key(master, *c), blob).is_err())
        .count();
    r.mfa_secrets_checked = secrets.len() as i64;
    if failed > 0 {
        r.problems.push(format!("{failed} of {} sampled two-factor secrets do not open with this master key", secrets.len()));
    }

    let links: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT customer_id, source_url FROM exposures WHERE source_url LIKE 'enc1:%' ORDER BY random() LIMIT $1",
    )
    .bind(sample)
    .fetch_all(pool)
    .await
    .context("sample sealed listing links")?;
    let failed = links
        .iter()
        .filter(|(c, sealed)| crate::services::url_vault::open(master, *c, sealed).is_err())
        .count();
    r.sealed_links_checked = links.len() as i64;
    if failed > 0 {
        r.problems.push(format!("{failed} of {} sampled listing links do not open with this master key", links.len()));
    }

    // A database that has customers but nothing to open would pass every check above for the
    // wrong reason. With customers present and no identity at all, say so rather than "usable".
    if r.customers > 0 && r.identities_checked == 0 && r.mfa_secrets_checked == 0 && r.sealed_links_checked == 0 {
        r.problems.push(
            "no encrypted data was found to test the master key against, so it is not proven to match this backup".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;

    fn master() -> MasterKey {
        MasterKey::from_base64(&ts::master_key_b64()).unwrap()
    }

    async fn populated(pool: &PgPool) -> Uuid {
        let c = ts::customer(pool, "personal", true).await;
        ts::identity(pool, &master(), c, "Jane Smith").await;
        crate::services::audit::AuditEntry::builder("user.login", "customer")
            .customer_id(c)
            .log(pool)
            .await
            .unwrap();
        c
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_good_restore_is_usable(pool: PgPool) {
        let c = populated(&pool).await;
        crate::services::mfa::begin_setup(&pool, &master(), c, "a@example.com").await.unwrap();
        let r = run(&pool, &master(), DEFAULT_SAMPLE).await.unwrap();
        assert!(r.ok(), "{}", r.render());
        assert_eq!((r.customers, r.identities_checked, r.mfa_secrets_checked), (1, 1, 1));
        assert_eq!(r.migrations_applied, r.migrations_expected);
        assert!(r.newest_audit_entry.is_some());
        assert!(r.render().contains("RESULT: usable"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_wrong_master_key_is_caught(pool: PgPool) {
        let c = populated(&pool).await;
        crate::services::mfa::begin_setup(&pool, &master(), c, "a@example.com").await.unwrap();
        let wrong = MasterKey::from_base64(&ts::other_master_key_b64()).unwrap();
        let r = run(&pool, &wrong, DEFAULT_SAMPLE).await.unwrap();
        assert!(!r.ok());
        assert!(r.problems.iter().any(|p| p.contains("identity profiles") && p.contains("master key")), "{:?}", r.problems);
        assert!(r.problems.iter().any(|p| p.contains("two-factor")), "{:?}", r.problems);
        assert!(r.render().contains("NOT usable"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn customers_with_nothing_to_decrypt_do_not_count_as_proof_of_the_key(pool: PgPool) {
        ts::customer(&pool, "free", false).await;
        let r = run(&pool, &master(), DEFAULT_SAMPLE).await.unwrap();
        assert!(!r.ok());
        assert!(r.problems.iter().any(|p| p.contains("not proven")), "{:?}", r.problems);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_empty_database_has_no_encrypted_data_and_no_complaint_about_it(pool: PgPool) {
        let r = run(&pool, &master(), DEFAULT_SAMPLE).await.unwrap();
        assert!(r.ok(), "{}", r.render());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_missing_or_altered_migration_is_caught(pool: PgPool) {
        populated(&pool).await;
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version = (SELECT max(version) FROM _sqlx_migrations)")
            .execute(&pool).await.unwrap();
        let r = run(&pool, &master(), DEFAULT_SAMPLE).await.unwrap();
        assert!(r.problems.iter().any(|p| p.contains("is not applied")), "{:?}", r.problems);

        sqlx::query("UPDATE _sqlx_migrations SET checksum = '\\x00'::bytea WHERE version = 1").execute(&pool).await.unwrap();
        let r = run(&pool, &master(), DEFAULT_SAMPLE).await.unwrap();
        assert!(r.problems.iter().any(|p| p.contains("different SQL")), "{:?}", r.problems);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_database_from_a_newer_version_is_caught(pool: PgPool) {
        populated(&pool).await;
        sqlx::query("INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (999999, 'future', true, '\\x01'::bytea, 0)")
            .execute(&pool).await.unwrap();
        let r = run(&pool, &master(), DEFAULT_SAMPLE).await.unwrap();
        assert!(r.problems.iter().any(|p| p.contains("999999") && p.contains("newer")), "{:?}", r.problems);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_database_that_is_not_ours_is_reported_not_crashed_on(pool: PgPool) {
        sqlx::query("DROP TABLE _sqlx_migrations").execute(&pool).await.unwrap();
        let r = run(&pool, &master(), DEFAULT_SAMPLE).await.unwrap();
        assert!(r.problems.iter().any(|p| p.contains("not a VectorScrub database")), "{:?}", r.problems);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn lost_audit_guards_are_caught_and_the_check_deletes_nothing(pool: PgPool) {
        populated(&pool).await;
        let r = run(&pool, &master(), DEFAULT_SAMPLE).await.unwrap();
        assert!(r.ok(), "{}", r.render());
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log").fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 1, "the probe is rolled back");

        sqlx::query("DROP TRIGGER audit_log_no_update_delete ON audit_log").execute(&pool).await.unwrap();
        let r = run(&pool, &master(), DEFAULT_SAMPLE).await.unwrap();
        assert!(r.problems.iter().any(|p| p.contains("audit_log") && p.contains("triggers")), "{:?}", r.problems);

        sqlx::query("DROP TRIGGER removal_audit_log_no_truncate ON removal_audit_log").execute(&pool).await.unwrap();
        let r = run(&pool, &master(), DEFAULT_SAMPLE).await.unwrap();
        assert!(r.problems.iter().any(|p| p.contains("removal_audit_log")), "{:?}", r.problems);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_report_contains_no_customer_data(pool: PgPool) {
        let c = populated(&pool).await;
        let email: String = sqlx::query_scalar("SELECT email FROM customers WHERE id = $1").bind(c).fetch_one(&pool).await.unwrap();
        let text = run(&pool, &master(), DEFAULT_SAMPLE).await.unwrap().render();
        for secret in [email.as_str(), "Jane", "Smith", &c.to_string(), "Main St"] {
            assert!(!text.contains(secret), "leaked {secret:?}");
        }
    }
}

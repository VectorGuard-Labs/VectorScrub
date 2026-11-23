//! Which database role the running service uses, and whether the schema is ready for it.
//!
//! The audit tables are protected by triggers that reject UPDATE, DELETE and TRUNCATE. Whoever
//! owns a table (or is a superuser) can drop those triggers. If the service itself connects as
//! the owner, a bug or a break-in that reaches arbitrary SQL can switch the audit trail off. So
//! production runs as two roles:
//!
//! * the **owner** (migrator) owns every object and runs `vectorscrub migrate`. Only operators
//!   and the one-shot `migrate` container hold its credentials.
//! * the **runtime** role connects the service. It can read and write rows and nothing else: it
//!   owns nothing, has no `TRIGGER` or `TRUNCATE` privilege and is not a superuser. See
//!   `infra/scripts/db_roles.sh`, which creates it.
//!
//! This module checks both halves at startup: the runtime role is not too powerful
//! (`check_runtime_role`), and when the service does not run migrations itself, every embedded
//! migration has already been applied (`pending_migrations`).

use sqlx::{migrate::Migrator, PgConnection};

/// The audit tables whose triggers must stay out of reach of the runtime role.
const GUARDED_TABLES: [&str; 2] = ["audit_log", "removal_audit_log"];

/// Ways the connected role is more powerful than the service needs. Empty means fine.
pub async fn runtime_role_problems(conn: &mut PgConnection) -> Result<Vec<String>, sqlx::Error> {
    let (superuser, bypass_rls): (bool, bool) =
        sqlx::query_as("SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user")
            .fetch_one(&mut *conn)
            .await?;
    let mut problems = Vec::new();
    if superuser {
        problems.push("is a superuser".to_string());
    }
    if bypass_rls {
        problems.push("bypasses row-level security".to_string());
    }
    for table in GUARDED_TABLES {
        let (owns, trigger, truncate): (bool, bool, bool) = sqlx::query_as(
            "SELECT pg_has_role(current_user, c.relowner, 'USAGE'), \
                    has_table_privilege(current_user, c.oid, 'TRIGGER'), \
                    has_table_privilege(current_user, c.oid, 'TRUNCATE') \
             FROM pg_class c WHERE c.oid = to_regclass($1)",
        )
        .bind(format!("public.{table}"))
        .fetch_one(&mut *conn)
        .await?;
        if owns {
            problems.push(format!("owns (or is a member of the owner of) {table}"));
        }
        if trigger {
            problems.push(format!("has the TRIGGER privilege on {table}"));
        }
        if truncate {
            problems.push(format!("has the TRUNCATE privilege on {table}"));
        }
    }
    Ok(problems)
}

/// Fail (production) or warn (anywhere else) when the runtime role is more powerful than needed.
pub async fn check_runtime_role(
    conn: &mut PgConnection,
    production: bool,
    allow_owner_role: bool,
) -> anyhow::Result<()> {
    let problems = runtime_role_problems(conn).await?;
    if problems.is_empty() {
        tracing::info!("Database role check passed: the service role cannot change the audit triggers");
        return Ok(());
    }
    let list = problems.join("; ");
    if production && !allow_owner_role {
        anyhow::bail!(
            "The database role in DATABASE_URL is more powerful than the service needs: it {list}. \
             That role could drop the audit-log triggers. Create a runtime role with infra/scripts/db_roles.sh, \
             run migrations separately with `vectorscrub migrate` (RUN_MIGRATIONS=false for the service), \
             and point DATABASE_URL at the runtime role. DATABASE_ALLOW_OWNER_ROLE=true overrides this and is a decision to record"
        );
    }
    tracing::warn!(
        problems = %list,
        "The database role used by the service can switch off the audit triggers; production refuses this (see docs/runbooks/database-roles.md)"
    );
    Ok(())
}

/// Versions of embedded migrations that are not applied (or whose recorded checksum differs).
/// Reads `_sqlx_migrations` only, so it works for a role that cannot create tables.
pub async fn pending_migrations(conn: &mut PgConnection, migrator: &Migrator) -> Result<Vec<i64>, sqlx::Error> {
    let table_exists: bool = sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NOT NULL")
        .fetch_one(&mut *conn)
        .await?;
    if !table_exists {
        return Ok(migrator.iter().filter(|m| !m.migration_type.is_down_migration()).map(|m| m.version).collect());
    }
    let applied: Vec<(i64, bool, Vec<u8>)> =
        sqlx::query_as("SELECT version, success, checksum FROM _sqlx_migrations")
            .fetch_all(&mut *conn)
            .await?;
    Ok(migrator
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .filter(|m| {
            !applied
                .iter()
                .any(|(v, ok, sum)| *v == m.version && *ok && sum.as_slice() == m.checksum.as_ref())
        })
        .map(|m| m.version)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    /// A role like the runtime role `infra/scripts/db_roles.sh` creates: row access, nothing else.
    async fn runtime_like_role(pool: &PgPool) -> String {
        let role = format!("vs_rt_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE ROLE {role} NOLOGIN")).execute(pool).await.unwrap();
        sqlx::query(&format!("GRANT USAGE ON SCHEMA public TO {role}")).execute(pool).await.unwrap();
        sqlx::query(&format!("GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO {role}"))
            .execute(pool)
            .await
            .unwrap();
        role
    }

    async fn drop_role(pool: &PgPool, role: &str) {
        sqlx::query(&format!("DROP OWNED BY {role}")).execute(pool).await.unwrap();
        sqlx::query(&format!("DROP ROLE {role}")).execute(pool).await.unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_owner_role_is_reported_and_a_row_only_role_is_not(pool: PgPool) {
        // The test database is made by a superuser that owns every table: the worst case.
        let mut conn = pool.acquire().await.unwrap();
        let problems = runtime_role_problems(&mut conn).await.unwrap();
        assert!(problems.iter().any(|p| p.contains("owns") && p.contains("audit_log")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("TRIGGER")), "{problems:?}");
        assert!(check_runtime_role(&mut conn, true, false).await.is_err(), "production refuses it");
        assert!(check_runtime_role(&mut conn, true, true).await.is_ok(), "unless explicitly allowed");
        assert!(check_runtime_role(&mut conn, false, false).await.is_ok(), "development only warns");
        drop(conn);

        let role = runtime_like_role(&pool).await;
        let mut conn = pool.acquire().await.unwrap();
        sqlx::query(&format!("SET ROLE {role}")).execute(&mut *conn).await.unwrap();
        let problems = runtime_role_problems(&mut conn).await.unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert!(check_runtime_role(&mut conn, true, false).await.is_ok());

        // And the point of it: that role cannot take the audit trail apart, while normal rows still work.
        for stmt in [
            "DROP TRIGGER IF EXISTS audit_log_no_update_delete ON audit_log",
            "ALTER TABLE audit_log DISABLE TRIGGER ALL",
            "ALTER TABLE removal_audit_log DISABLE TRIGGER ALL",
            "DROP TABLE audit_log",
            "TRUNCATE audit_log",
            "CREATE TABLE should_not_exist (x int)",
        ] {
            let err = sqlx::query(stmt).execute(&mut *conn).await.unwrap_err().to_string();
            assert!(err.contains("must be owner") || err.contains("permission denied"), "{stmt}: {err}");
        }
        sqlx::query("SELECT count(*) FROM customers").fetch_one(&mut *conn).await.unwrap();
        sqlx::query("RESET ROLE").execute(&mut *conn).await.unwrap();
        drop(conn);
        drop_role(&pool, &role).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_role_with_the_trigger_privilege_is_reported(pool: PgPool) {
        let role = runtime_like_role(&pool).await;
        sqlx::query(&format!("GRANT TRIGGER ON audit_log TO {role}")).execute(&pool).await.unwrap();
        let mut conn = pool.acquire().await.unwrap();
        sqlx::query(&format!("SET ROLE {role}")).execute(&mut *conn).await.unwrap();
        let problems = runtime_role_problems(&mut conn).await.unwrap();
        assert_eq!(problems, vec!["has the TRIGGER privilege on audit_log".to_string()]);
        sqlx::query("RESET ROLE").execute(&mut *conn).await.unwrap();
        drop(conn);
        drop_role(&pool, &role).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn pending_migrations_are_found_without_creating_anything(pool: PgPool) {
        let migrator = sqlx::migrate!("./migrations");
        let mut conn = pool.acquire().await.unwrap();
        assert!(pending_migrations(&mut conn, &migrator).await.unwrap().is_empty());

        sqlx::query("DELETE FROM _sqlx_migrations WHERE version = (SELECT max(version) FROM _sqlx_migrations)")
            .execute(&mut *conn)
            .await
            .unwrap();
        assert_eq!(pending_migrations(&mut conn, &migrator).await.unwrap().len(), 1);

        sqlx::query("UPDATE _sqlx_migrations SET checksum = '\\x00' WHERE version = 1").execute(&mut *conn).await.unwrap();
        assert!(pending_migrations(&mut conn, &migrator).await.unwrap().contains(&1), "a changed migration is not 'applied'");
    }
}

//! Verifying the audit hash chain (migration 0025).
//!
//! Every audit row written after that migration has a link in `audit_chain` whose hash covers the
//! previous link's hash and the row's immutable fields (what happened, to which resource, with what
//! result, and when). `verify` recomputes the chain from the audit tables and reports the first
//! place it does not hold. `head` is the hash to keep somewhere the database host cannot write:
//! with it, even someone who rewrites the whole chain consistently is caught, because the head they
//! leave behind differs from the one you stored. Procedure: `docs/runbooks/audit-chain.md`.

use sha2::{Digest, Sha256};
use sqlx::PgPool;

#[derive(Debug, PartialEq, Eq)]
pub struct ChainReport {
    /// Links checked.
    pub links: usize,
    /// Sequence number and hash of the last link, if there is one.
    pub head: Option<(i64, String)>,
    /// The first problem found, if any.
    pub problem: Option<String>,
}

fn link_hash(prev: &[u8], fields: &str) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(prev);
    h.update(fields.as_bytes());
    h.finalize().to_vec()
}

/// Recompute the whole chain. Reads only; works for the runtime role.
pub async fn verify(pool: &PgPool) -> Result<ChainReport, sqlx::Error> {
    let rows: Vec<(i64, String, Vec<u8>, Option<String>)> = sqlx::query_as(
        "SELECT seq, source, link_hash, audit_chain_fields(source, row_id) FROM audit_chain ORDER BY seq",
    )
    .fetch_all(pool)
    .await?;

    let mut prev: Vec<u8> = Vec::new();
    let mut checked = 0usize;
    let mut head = None;
    for (seq, source, stored, fields) in rows {
        let Some(fields) = fields else {
            return Ok(ChainReport {
                links: checked,
                head,
                problem: Some(format!("link {seq}: the {source} row it covers is missing (deleted?)")),
            });
        };
        if link_hash(&prev, &fields) != stored {
            return Ok(ChainReport {
                links: checked,
                head,
                problem: Some(format!(
                    "link {seq}: the hash does not match (a {source} row, or an earlier link, was changed or removed)"
                )),
            });
        }
        checked += 1;
        head = Some((seq, hex::encode(&stored)));
        prev = stored;
    }

    // Rows that should have a link and do not: written with the chain trigger switched off.
    let unchained: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM audit_log a WHERE NOT EXISTS (SELECT 1 FROM audit_chain c WHERE c.source = 'audit_log' AND c.row_id = a.id)
                   AND a.created_at >= (SELECT min(a2.created_at) FROM audit_log a2 JOIN audit_chain c2 ON c2.source = 'audit_log' AND c2.row_id = a2.id))
              + (SELECT count(*) FROM removal_audit_log r WHERE NOT EXISTS (SELECT 1 FROM audit_chain c WHERE c.source = 'removal_audit_log' AND c.row_id = r.id)
                   AND r.recorded_at >= (SELECT min(r2.recorded_at) FROM removal_audit_log r2 JOIN audit_chain c2 ON c2.source = 'removal_audit_log' AND c2.row_id = r2.id))",
    )
    .fetch_one(pool)
    .await?;
    let problem = (unchained > 0).then(|| format!("{unchained} audit row(s) written after the chain began have no link"));
    Ok(ChainReport { links: checked, head, problem })
}

/// Does the chain still contain link `seq` with exactly this hash? A head stored earlier must stay in
/// the chain forever; if it is gone or different, the chain was cut or rewritten after it was stored.
pub async fn contains_head(pool: &PgPool, seq: i64, hash_hex: &str) -> Result<bool, sqlx::Error> {
    let Ok(want) = hex::decode(hash_hex.trim()) else { return Ok(false) };
    let stored: Option<Vec<u8>> = sqlx::query_scalar("SELECT link_hash FROM audit_chain WHERE seq = $1")
        .bind(seq)
        .fetch_optional(pool)
        .await?;
    Ok(stored.is_some_and(|h| h == want))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn log(pool: &PgPool, action: &str) {
        sqlx::query("INSERT INTO audit_log (action, resource_type) VALUES ($1, 'customer')")
            .bind(action)
            .execute(pool)
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_untouched_chain_verifies_and_grows_with_every_audit_row(pool: PgPool) {
        let empty = verify(&pool).await.unwrap();
        assert_eq!((empty.links, empty.head, empty.problem), (0, None, None));
        for a in ["user.register", "user.login", "scan.initiate"] {
            log(&pool, a).await;
        }
        sqlx::query("INSERT INTO removal_audit_log (action, actor) VALUES ('authorized', 'customer')")
            .execute(&pool)
            .await
            .unwrap();
        let r = verify(&pool).await.unwrap();
        assert_eq!((r.links, r.problem.as_deref()), (4, None));
        let (_, head) = r.head.unwrap();
        assert_eq!(head.len(), 64);
        log(&pool, "scan.failed").await;
        let r2 = verify(&pool).await.unwrap();
        assert_eq!(r2.links, 5);
        assert_ne!(r2.head.unwrap().1, head, "the head moves with every row");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_owner_who_edits_a_row_with_the_triggers_off_is_caught(pool: PgPool) {
        for a in ["user.register", "user.login", "identity.release", "scan.initiate"] {
            log(&pool, a).await;
        }
        assert!(verify(&pool).await.unwrap().problem.is_none());

        // What an attacker with the owner role can do to the append-only table...
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("ALTER TABLE audit_log DISABLE TRIGGER audit_log_no_update_delete").execute(&mut *tx).await.unwrap();
        sqlx::query("UPDATE audit_log SET action = 'user.login' WHERE action = 'identity.release'").execute(&mut *tx).await.unwrap();
        sqlx::query("ALTER TABLE audit_log ENABLE TRIGGER audit_log_no_update_delete").execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        // ...is found by the chain.
        let r = verify(&pool).await.unwrap();
        assert!(r.problem.as_deref().unwrap().contains("does not match"), "{r:?}");

    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_owner_who_deletes_a_row_or_a_tail_is_caught(pool: PgPool) {
        for a in ["user.register", "user.login", "scan.initiate"] {
            log(&pool, a).await;
        }
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("ALTER TABLE audit_log DISABLE TRIGGER audit_log_no_update_delete").execute(&mut *tx).await.unwrap();
        sqlx::query("DELETE FROM audit_log WHERE action = 'user.login'").execute(&mut *tx).await.unwrap();
        sqlx::query("ALTER TABLE audit_log ENABLE TRIGGER audit_log_no_update_delete").execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        assert!(verify(&pool).await.unwrap().problem.unwrap().contains("missing"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_head_changes_if_the_tail_of_the_chain_is_cut_off(pool: PgPool) {
        // Cutting links off the end leaves a chain that still verifies. That is what a stored head
        // is for: it no longer matches.
        for a in ["user.register", "user.login", "scan.initiate"] {
            log(&pool, a).await;
        }
        let before = verify(&pool).await.unwrap().head.unwrap();
        assert!(contains_head(&pool, before.0, &before.1).await.unwrap(), "before the cut the stored head is present");
        assert!(!contains_head(&pool, before.0, "00").await.unwrap(), "a different hash is not");
        assert!(!contains_head(&pool, before.0, "not hex").await.unwrap());
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("ALTER TABLE audit_chain DISABLE TRIGGER audit_chain_no_update_delete").execute(&mut *tx).await.unwrap();
        sqlx::query("DELETE FROM audit_chain WHERE seq = (SELECT max(seq) FROM audit_chain)").execute(&mut *tx).await.unwrap();
        sqlx::query("ALTER TABLE audit_chain ENABLE TRIGGER audit_chain_no_update_delete").execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        assert!(!contains_head(&pool, before.0, &before.1).await.unwrap(), "after the cut the stored head is gone");
        let after = verify(&pool).await.unwrap();
        assert_ne!(after.head.unwrap(), before, "the stored head exposes the cut");
        assert!(after.problem.unwrap().contains("no link"), "and the row without a link is reported too");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_row_written_with_the_chain_trigger_off_is_reported(pool: PgPool) {
        log(&pool, "user.register").await;
        sqlx::query("ALTER TABLE audit_log DISABLE TRIGGER audit_log_chain").execute(&pool).await.unwrap();
        log(&pool, "sneaky").await;
        sqlx::query("ALTER TABLE audit_log ENABLE TRIGGER audit_log_chain").execute(&pool).await.unwrap();
        let r = verify(&pool).await.unwrap();
        assert!(r.problem.unwrap().contains("no link"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_chain_cannot_be_edited_and_erasing_an_account_does_not_break_it(pool: PgPool) {
        let customer = crate::test_support::customer(&pool, "free", false).await;
        sqlx::query("INSERT INTO audit_log (action, resource_type, customer_id, user_email, ip_address) VALUES ('user.login', 'customer', $1, 'a@example.com', '1.2.3.4')")
            .bind(customer)
            .execute(&pool)
            .await
            .unwrap();
        log(&pool, "user.login").await;
        for stmt in ["DELETE FROM audit_chain", "UPDATE audit_chain SET link_hash = '\\x00'", "TRUNCATE audit_chain"] {
            let e = sqlx::query(stmt).execute(&pool).await.unwrap_err().to_string();
            assert!(e.contains("append-only"), "{stmt}: {e}");
        }
        crate::services::account_deletion::delete_account(&pool, customer).await.unwrap();
        let r = verify(&pool).await.unwrap();
        assert!(r.problem.is_none(), "{r:?}");
        assert!(r.links >= 2);
    }
}

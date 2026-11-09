use anyhow::{anyhow, Result};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::models::customer::{Customer, SubscriptionTier};
use crate::models::family::{FamilyMember, FamilyMemberResponse, FamilyStats};

const MAX_FAMILY_MEMBERS: usize = 4;

/// Add a family member to a family plan.
/// Requires owner to have 'family' subscription tier.
pub async fn add_family_member(
    pool: &PgPool,
    owner_id: Uuid,
    member_email: &str,
) -> Result<FamilyMember> {
    // Verify owner has family plan
    let owner: Customer = sqlx::query_as(
        "SELECT * FROM customers WHERE id = $1"
    )
    .bind(owner_id)
    .fetch_one(pool)
    .await?;

    if owner.subscription_tier != SubscriptionTier::Family {
        return Err(anyhow!("Only family plan subscribers can add family members"));
    }

    // Check member limit
    let current_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM family_members WHERE owner_id = $1"
    )
    .bind(owner_id)
    .fetch_one(pool)
    .await?;

    if current_count >= MAX_FAMILY_MEMBERS as i64 {
        return Err(anyhow!("Family plan limited to {} members", MAX_FAMILY_MEMBERS));
    }

    // Find member by email
    let member: Option<Customer> = sqlx::query_as(
        "SELECT * FROM customers WHERE email = $1"
    )
    .bind(member_email)
    .fetch_optional(pool)
    .await?;

    let member_id = if let Some(m) = member {
        m.id
    } else {
        return Err(anyhow!("Member not found. They must create an account first."));
    };

    // Check if already a member
    let existing: Option<FamilyMember> = sqlx::query_as(
        "SELECT * FROM family_members WHERE owner_id = $1 AND member_id = $2"
    )
    .bind(owner_id)
    .bind(member_id)
    .fetch_optional(pool)
    .await?;

    if existing.is_some() {
        return Err(anyhow!("This person is already a family member"));
    }

    // Add family member
    let family_member = sqlx::query_as::<_, FamilyMember>(
        r#"
        INSERT INTO family_members (owner_id, member_id, role, invited_at, created_at)
        VALUES ($1, $2, 'member', NOW(), NOW())
        RETURNING *
        "#
    )
    .bind(owner_id)
    .bind(member_id)
    .fetch_one(pool)
    .await?;

    tracing::info!(
        owner_id = %owner_id,
        member_id = %member_id,
        "Family member added"
    );

    Ok(family_member)
}

/// Remove a family member.
pub async fn remove_family_member(
    pool: &PgPool,
    owner_id: Uuid,
    member_id: Uuid,
) -> Result<()> {
    let result = sqlx::query(
        "DELETE FROM family_members WHERE owner_id = $1 AND member_id = $2"
    )
    .bind(owner_id)
    .bind(member_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(anyhow!("Family member not found"));
    }

    tracing::info!(
        owner_id = %owner_id,
        member_id = %member_id,
        "Family member removed"
    );

    Ok(())
}

/// List all family members for an owner.
pub async fn list_family_members(
    pool: &PgPool,
    owner_id: Uuid,
) -> Result<Vec<FamilyMemberResponse>> {
    let members = sqlx::query_as::<_, (Uuid, Uuid, String, Option<OffsetDateTime>, Option<OffsetDateTime>, String, Option<f32>)>(
        r#"
        SELECT
            fm.id,
            fm.member_id,
            fm.role,
            fm.invited_at,
            fm.accepted_at,
            c.email,
            c.privacy_score
        FROM family_members fm
        JOIN customers c ON c.id = fm.member_id
        WHERE fm.owner_id = $1
        ORDER BY fm.created_at ASC
        "#
    )
    .bind(owner_id)
    .fetch_all(pool)
    .await?;

    let responses: Vec<FamilyMemberResponse> = members
        .into_iter()
        .map(|(id, _member_id, role, invited_at, accepted_at, email, privacy_score)| {
            FamilyMemberResponse {
                id,
                member_email: email,
                member_name: None, // Could be fetched from identity_profiles if needed
                privacy_score,
                role,
                invited_at,
                accepted_at,
            }
        })
        .collect();

    Ok(responses)
}

/// Get aggregate family statistics.
pub async fn get_family_stats(
    pool: &PgPool,
    owner_id: Uuid,
) -> Result<FamilyStats> {
    let members = list_family_members(pool, owner_id).await?;

    // Calculate aggregate stats
    let total_members = members.len();
    let scores: Vec<f32> = members.iter().filter_map(|m| m.privacy_score).collect();
    let average_privacy_score = if scores.is_empty() {
        100.0
    } else {
        scores.iter().sum::<f32>() / scores.len() as f32
    };

    // Count total exposures across all family members
    let member_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT member_id FROM family_members WHERE owner_id = $1"
    )
    .bind(owner_id)
    .fetch_all(pool)
    .await?;

    let mut total_exposures = 0i64;
    for member_id in &member_ids {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM exposures WHERE customer_id = $1 AND is_active = true"
        )
        .bind(member_id)
        .fetch_one(pool)
        .await?;
        total_exposures += count;
    }

    Ok(FamilyStats {
        total_members,
        average_privacy_score,
        total_exposures,
        members,
    })
}

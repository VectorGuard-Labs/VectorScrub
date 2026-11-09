use anyhow::{anyhow, Result};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::family::{FamilyMember, FamilyMemberResponse, FamilyStats};

const MAX_FAMILY_MEMBERS: usize = 5;

/// Add a family member - DISABLED: All plans are now per person only.
pub async fn add_family_member(
    pool: &PgPool,
    owner_id: Uuid,
    member_email: &str,
) -> Result<FamilyMember> {
    let _pool = pool;
    let _owner_id = owner_id;
    let _member_email = member_email;
    return Err(anyhow!("Family member management is no longer available. All plans are per person."));

}

/// Remove a family member - DISABLED: All plans are now per person only.
pub async fn remove_family_member(
    pool: &PgPool,
    owner_id: Uuid,
    member_id: Uuid,
) -> Result<()> {
    let _pool = pool;
    let _owner_id = owner_id;
    let _member_id = member_id;
    return Err(anyhow!("Family member management is no longer available. All plans are per person."));
}

/// List all family members - DISABLED: All plans are now per person only.
pub async fn list_family_members(
    pool: &PgPool,
    owner_id: Uuid,
) -> Result<Vec<FamilyMemberResponse>> {
    let _pool = pool;
    let _owner_id = owner_id;
    return Err(anyhow!("Family member management is no longer available. All plans are per person."));
}

/// Get aggregate family statistics - DISABLED: All plans are now per person only.
pub async fn get_family_stats(
    pool: &PgPool,
    owner_id: Uuid,
) -> Result<FamilyStats> {
    let _pool = pool;
    let _owner_id = owner_id;
    return Err(anyhow!("Family member management is no longer available. All plans are per person."));
}

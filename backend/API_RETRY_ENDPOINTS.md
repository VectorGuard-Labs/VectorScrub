## Removal Retry API Endpoints

Backend API endpoints to support customer-initiated retries of failed removal requests.

### 1. Check Retry Eligibility

Check if a removal request can be retried.

```
GET /api/v1/customers/{customer_id}/removal-requests/{request_id}/retry-eligibility
```

**Headers:**
```
Authorization: Bearer {jwt_token}
```

**Response:**
```json
{
  "allowed": true,
  "reason": "Retry allowed",
  "retry_count": 1,
  "max_retries": 3,
  "cooldown_until": null,
  "daily_limit_reached": false
}
```

**Implementation:**
```rust
// In backend/src/handlers/removal.rs

pub async fn check_retry_eligibility(
    State(app_state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((customer_id, request_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<RetryEligibilityResponse>, AppError> {
    // Verify customer_id matches JWT claims
    if claims.customer_id != customer_id {
        return Err(AppError::Unauthorized);
    }

    // Call PostgreSQL function: can_retry_removal()
    let eligibility = sqlx::query_as!(
        RetryEligibility,
        "SELECT * FROM can_retry_removal($1, $2)",
        request_id,
        customer_id
    )
    .fetch_one(&app_state.db)
    .await?;

    Ok(Json(RetryEligibilityResponse {
        allowed: eligibility.allowed,
        reason: eligibility.reason,
        retry_count: eligibility.retry_count,
        max_retries: eligibility.max_retries,
        cooldown_until: eligibility.cooldown_until,
        daily_limit_reached: eligibility.daily_limit_reached,
    }))
}
```

### 2. Initiate Retry

Create a retry attempt for a failed removal request.

```
POST /api/v1/customers/{customer_id}/removal-requests/{request_id}/retry
```

**Headers:**
```
Authorization: Bearer {jwt_token}
Content-Type: application/json
```

**Request Body:**
```json
{
  "initiated_by": "customer"
}
```

**Response:**
```json
{
  "success": true,
  "retry_history_id": "550e8400-e29b-41d4-a716-446655440000",
  "message": "Retry initiated successfully"
}
```

**Implementation:**
```rust
pub async fn initiate_retry(
    State(app_state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path((customer_id, request_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<InitiateRetryRequest>,
) -> Result<Json<InitiateRetryResponse>, AppError> {
    // Verify customer_id matches JWT claims
    if claims.customer_id != customer_id {
        return Err(AppError::Unauthorized);
    }

    // Call PostgreSQL function: initiate_removal_retry()
    let retry_history_id = sqlx::query_scalar!(
        "SELECT initiate_removal_retry($1, $2, $3)",
        request_id,
        customer_id,
        payload.initiated_by.as_deref().unwrap_or("customer")
    )
    .fetch_one(&app_state.db)
    .await?;

    // Audit log
    audit_log(
        &app_state.db,
        customer_id,
        "removal_retry_initiated",
        json!({
            "removal_request_id": request_id,
            "retry_history_id": retry_history_id,
        })
    ).await?;

    Ok(Json(InitiateRetryResponse {
        success: true,
        retry_history_id: Some(retry_history_id),
        message: "Retry initiated successfully".to_string(),
    }))
}
```

### 3. Get Pending Retries (Internal Agent Endpoint)

Fetch pending retry requests for the agent worker to process.

```
GET /api/internal/removal-retries/pending?limit=100
```

**Headers:**
```
X-Agent-Key: {agent_secret}
```

**Response:**
```json
{
  "retries": [
    {
      "retry_history_id": "550e8400-e29b-41d4-a716-446655440000",
      "removal_request_id": "660e8400-e29b-41d4-a716-446655440001",
      "customer_id": "770e8400-e29b-41d4-a716-446655440002",
      "retry_number": 2,
      "initiated_at": "2026-11-13T10:30:00Z"
    }
  ]
}
```

**Implementation:**
```rust
// In backend/src/handlers/internal.rs

pub async fn get_pending_retries(
    State(app_state): State<AppState>,
    Extension(agent_auth): Extension<AgentAuth>,
    Query(params): Query<PendingRetriesQuery>,
) -> Result<Json<PendingRetriesResponse>, AppError> {
    let limit = params.limit.unwrap_or(100).min(500);

    let retries = sqlx::query_as!(
        PendingRetry,
        r#"
        SELECT
            rh.id as retry_history_id,
            rh.removal_request_id,
            rh.customer_id,
            rh.retry_number,
            rh.initiated_at
        FROM removal_retry_history rh
        WHERE rh.status = 'pending'
        AND rh.initiated_at >= NOW() - INTERVAL '24 hours'
        ORDER BY rh.initiated_at ASC
        LIMIT $1
        "#,
        limit as i64
    )
    .fetch_all(&app_state.db)
    .await?;

    Ok(Json(PendingRetriesResponse {
        retries,
    }))
}
```

### 4. Update Retry History (Internal Agent Endpoint)

Update retry history after execution completes.

```
PATCH /api/internal/removal-retries/{retry_history_id}
```

**Headers:**
```
X-Agent-Key: {agent_secret}
Content-Type: application/json
```

**Request Body:**
```json
{
  "status": "success",
  "confirmation_ref": "REF-123456",
  "evidence_hash": "abc123...",
  "error_message": null
}
```

**Response:**
```json
{
  "success": true
}
```

**Implementation:**
```rust
pub async fn update_retry_history(
    State(app_state): State<AppState>,
    Extension(agent_auth): Extension<AgentAuth>,
    Path(retry_history_id): Path<Uuid>,
    Json(payload): Json<UpdateRetryHistoryRequest>,
) -> Result<Json<UpdateRetryHistoryResponse>, AppError> {
    sqlx::query!(
        r#"
        UPDATE removal_retry_history
        SET
            status = $2,
            completed_at = NOW(),
            confirmation_ref = $3,
            evidence_hash = $4,
            error_message = $5
        WHERE id = $1
        "#,
        retry_history_id,
        payload.status,
        payload.confirmation_ref,
        payload.evidence_hash,
        payload.error_message
    )
    .execute(&app_state.db)
    .await?;

    // Also update the main removal_request if succeeded
    if payload.status == "success" {
        sqlx::query!(
            r#"
            UPDATE removal_requests
            SET
                status = 'submitted',
                confirmation_ref = $2,
                evidence_hash = $3,
                submitted_at = NOW()
            WHERE id = (
                SELECT removal_request_id
                FROM removal_retry_history
                WHERE id = $1
            )
            "#,
            retry_history_id,
            payload.confirmation_ref,
            payload.evidence_hash
        )
        .execute(&app_state.db)
        .await?;
    }

    Ok(Json(UpdateRetryHistoryResponse {
        success: true,
    }))
}
```

### 5. Get Removal Request Details (Internal Agent Endpoint)

Fetch full details of a removal request for retry execution.

```
GET /api/internal/removal-requests/{request_id}?customer_id={customer_id}
```

**Headers:**
```
X-Agent-Key: {agent_secret}
```

**Response:**
```json
{
  "removal_request_id": "660e8400-e29b-41d4-a716-446655440001",
  "customer_id": "770e8400-e29b-41d4-a716-446655440002",
  "source_url": "https://spokeo.com/profile/...",
  "procedure": {
    "source_name": "spokeo.com",
    "procedure_type": "web_form",
    "opt_out_url": "https://www.spokeo.com/optout",
    "opt_out_email": null,
    "instructions": "...",
    "requires_id_verification": false,
    "estimated_days": 7
  },
  "identity": {
    "full_name": "...",
    "email": "...",
    "current_address": "...",
    "phone_numbers": []
  }
}
```

**Implementation:**
```rust
pub async fn get_removal_request_for_retry(
    State(app_state): State<AppState>,
    Extension(agent_auth): Extension<AgentAuth>,
    Path(request_id): Path<Uuid>,
    Query(params): Query<GetRemovalRequestQuery>,
) -> Result<Json<RemovalRequestDetails>, AppError> {
    // Decrypt and return full removal request details
    // This is privileged - only accessible to internal agents
    // Implementation similar to existing get_identity_for_scan
    // ...
}
```

### Data Models

```rust
#[derive(Serialize, Deserialize)]
pub struct RetryEligibilityResponse {
    pub allowed: bool,
    pub reason: String,
    pub retry_count: i32,
    pub max_retries: i32,
    pub cooldown_until: Option<DateTime<Utc>>,
    pub daily_limit_reached: bool,
}

#[derive(Deserialize)]
pub struct InitiateRetryRequest {
    pub initiated_by: Option<String>,
}

#[derive(Serialize)]
pub struct InitiateRetryResponse {
    pub success: bool,
    pub retry_history_id: Option<Uuid>,
    pub message: String,
}

#[derive(Serialize)]
pub struct PendingRetry {
    pub retry_history_id: Uuid,
    pub removal_request_id: Uuid,
    pub customer_id: Uuid,
    pub retry_number: i32,
    pub initiated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct UpdateRetryHistoryRequest {
    pub status: String, // 'success' or 'failed'
    pub confirmation_ref: Option<String>,
    pub evidence_hash: Option<String>,
    pub error_message: Option<String>,
}
```

### Rate Limiting

Implement rate limiting middleware:

```rust
// Max 10 retry initiations per customer per day (enforced by database)
// Max 1 retry per removal request per hour (enforced by cooldown)
// Max 3 retries per removal request total (enforced by max_retries)
```

### Security

1. **Customer Endpoints** - Require valid JWT, verify customer_id matches token
2. **Internal Endpoints** - Require X-Agent-Key header validation
3. **Audit Logging** - Log all retry initiations
4. **PII Encryption** - Identity data returned to agents must be decrypted server-side

### Testing

```bash
# Test eligibility check
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:3000/api/v1/customers/$CUSTOMER_ID/removal-requests/$REQUEST_ID/retry-eligibility

# Initiate retry
curl -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"initiated_by":"customer"}' \
  http://localhost:3000/api/v1/customers/$CUSTOMER_ID/removal-requests/$REQUEST_ID/retry

# Get pending retries (internal)
curl -H "X-Agent-Key: $AGENT_KEY" \
  http://localhost:3000/api/internal/removal-retries/pending?limit=10
```

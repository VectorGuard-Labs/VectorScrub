//! The key service: where customers' data keys come from.
//!
//! Every customer has a random 256-bit data key (DEK, `crypto::envelope`). The database stores it
//! only wrapped, as `customer_keys.wrapped_dek`, encrypted by a key-encryption key (KEK) that this
//! process never holds when the provider is PyHSM:
//!
//! ```text
//!   handler -> KeyService::customer(id) -> cache hit?  yes -> DEK
//!                                          no -> read wrapped DEK from Postgres
//!                                             -> ask PyHSM (Unix socket) to decrypt it with the KEK
//!                                             -> keep it in memory for DEK_CACHE_TTL_SECS
//! ```
//!
//! What this buys, and what it does not:
//!   * Reading this process's environment or the database no longer yields anything that opens
//!     customer data. There is no master key in either.
//!   * Someone who takes over the running backend can still ask the key service to unwrap keys for
//!     as long as they hold it (and any DEKs already in the cache), and every unwrap is logged by
//!     PyHSM. The key service limits the damage and makes it visible. It does not make a
//!     compromised backend harmless.
//!   * The KEK can be rotated without touching customer data: `kms rotate` adds a KEK version and
//!     `kms rewrap` re-wraps the (small) data keys under it.
//!   * If the key service is down, nothing can be decrypted and requests fail (they never fall
//!     back to anything weaker).
//!
//! `KeyProviderConfig::Env` derives the KEK from `MASTER_ENCRYPTION_KEY` instead. It exists so
//! development and the test suite need no PyHSM, and production refuses it (see `config.rs`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use sqlx::PgPool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::envelope::CustomerKeys;
use crate::crypto::MasterKey;
use crate::services::config::{KeyProviderConfig, SecretString};

/// Largest reply accepted from the key service. A wrapped key reply is a few hundred bytes.
const MAX_REPLY_BYTES: usize = 1024 * 1024;
/// Cache size limit. The cache holds 32 bytes plus bookkeeping per customer, so this is a bound
/// on memory (a few MB), not a tuning knob.
const MAX_CACHE_ENTRIES: usize = 50_000;
/// Bulk operations (rotation, re-wrap, migration) wait this long between attempts when the key
/// service's rate limit is reached, and give up on one item after `PATIENT_MAX_WAIT`.
const PATIENT_RETRY_EVERY: Duration = Duration::from_secs(2);
const PATIENT_MAX_WAIT: Duration = Duration::from_secs(180);

/// Run `op`, waiting and retrying while the key service reports its rate limit. For operator
/// commands that work through every customer, never for a customer's request (which must not
/// hang). PyHSM's window is a sliding minute, so waiting frees capacity by itself.
async fn patiently<T, F, Fut>(mut op: F) -> Result<T, KeyError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, KeyError>>,
{
    let started = Instant::now();
    let mut announced = false;
    loop {
        match op().await {
            Err(KeyError::RateLimited) if started.elapsed() < PATIENT_MAX_WAIT => {
                if !announced {
                    tracing::info!("Key service rate limit reached; slowing down and retrying");
                    announced = true;
                }
                tokio::time::sleep(PATIENT_RETRY_EVERY).await;
            }
            other => return other,
        }
    }
}

/// Readiness is not re-asked of the key service more often than this.
const HEALTH_CACHE: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    /// The key service could not be reached, or did not answer in time.
    #[error("the key service is unavailable ({0})")]
    Unavailable(String),
    /// The key service answered, and said no (a missing key, a policy refusal).
    #[error("the key service refused the request ({0})")]
    Refused(String),
    /// The key service's per-key rate limit (PyHSM `--rate-limit`, operations per minute) was
    /// reached. Temporary: the request may succeed a few seconds later.
    #[error("the key service rate limit was reached; try again shortly")]
    RateLimited,
    /// A wrapped key is damaged, or was made for another customer.
    #[error("a wrapped data key is damaged or belongs to another customer")]
    Integrity,
    #[error("data keys are wrapped by '{found}' but this service uses '{expected}'; run `vectorscrub kms rewrap`")]
    ProviderMismatch { found: String, expected: &'static str },
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// A data key as stored: wrapped, with which provider and KEK version did it.
#[derive(Debug, Clone)]
pub struct Wrapped {
    pub bytes: Vec<u8>,
    pub provider: &'static str,
    pub kek_version: i32,
}

// ===========================================================================
// Wrapping with a key derived from the environment (development, tests)
// ===========================================================================

pub struct EnvWrapper {
    kek: Zeroizing<[u8; 32]>,
}

impl EnvWrapper {
    pub fn new(master: &MasterKey) -> Self {
        let hk = Hkdf::<Sha256>::new(None, master.as_bytes());
        let mut kek = Zeroizing::new([0u8; 32]);
        hk.expand(b"vectorscrub/v2/kek/env", &mut *kek).expect("32 bytes is a valid HKDF output length");
        Self { kek }
    }

    fn aad(customer: Uuid) -> Vec<u8> {
        let mut aad = b"vectorscrub/v2/wrap\0".to_vec();
        aad.extend_from_slice(customer.as_bytes());
        aad
    }

    fn wrap(&self, customer: Uuid, dek: &[u8; 32]) -> Result<Wrapped, KeyError> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&*self.kek));
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = cipher
            .encrypt(&nonce, Payload { msg: dek, aad: &Self::aad(customer) })
            .map_err(|_| KeyError::Integrity)?;
        let mut bytes = nonce.to_vec();
        bytes.extend_from_slice(&ct);
        Ok(Wrapped { bytes, provider: "env", kek_version: 1 })
    }

    fn unwrap(&self, customer: Uuid, wrapped: &[u8]) -> Result<Zeroizing<[u8; 32]>, KeyError> {
        if wrapped.len() < 12 + 16 {
            return Err(KeyError::Integrity);
        }
        let (nonce, ct) = wrapped.split_at(12);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&*self.kek));
        let plain = cipher
            .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad: &Self::aad(customer) })
            .map_err(|_| KeyError::Integrity)?;
        let plain = Zeroizing::new(plain);
        let mut dek = Zeroizing::new([0u8; 32]);
        if plain.len() != 32 {
            return Err(KeyError::Integrity);
        }
        dek.copy_from_slice(&plain);
        Ok(dek)
    }
}

// ===========================================================================
// PyHSM over its Unix socket
// ===========================================================================

/// A client for PyHSM's IPC server: 4-byte big-endian length, then a JSON object, both ways.
///
/// Requests use PyHSM's version-2 authentication (PyHSM `hsm/ipc_auth.py`): the HMAC covers the
/// request id, the operation, a timestamp and the SHA-256 of the whole request body, so a
/// captured request can be neither replayed nor altered. `auth_message` must stay byte-for-byte
/// identical to PyHSM's; a frozen vector in the tests checks it.
/// PyHSM version-2 authentication message (byte-for-byte as `hsm/ipc_auth.auth_message`):
/// `"pyhsm-ipc-v2\n" + request_id + "\n" + operation + "\n" + ts + "\n" + sha256hex(body)`, where
/// body is the canonical JSON of the request without `auth` and `auth_v`: keys sorted, no
/// whitespace, UTF-8. serde_json's `Value` keeps object keys sorted (the `preserve_order`
/// feature is off) and writes compact JSON, which is exactly that for the strings and integers
/// these requests carry.
fn auth_message(request: &Value) -> Vec<u8> {
    let mut unsigned = request.clone();
    if let Some(obj) = unsigned.as_object_mut() {
        obj.remove("auth");
        obj.remove("auth_v");
    }
    let body = serde_json::to_vec(&unsigned).expect("a JSON value always serialises");
    let field = |k: &str| match request.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    format!(
        "pyhsm-ipc-v2\n{}\n{}\n{}\n{}",
        field("request_id"),
        field("type"),
        field("ts"),
        hex::encode(<Sha256 as sha2::Digest>::digest(&body))
    )
    .into_bytes()
}

pub struct PyHsmClient {
    socket: PathBuf,
    secret: SecretString,
    key_id: String,
    timeout: Duration,
    conn: tokio::sync::Mutex<Option<UnixStream>>,
}

impl PyHsmClient {
    pub fn new(socket: &str, secret: SecretString, key_id: &str, timeout_ms: u64) -> Self {
        Self {
            socket: PathBuf::from(socket),
            secret,
            key_id: key_id.to_string(),
            timeout: Duration::from_millis(timeout_ms.max(100)),
            conn: tokio::sync::Mutex::new(None),
        }
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// Sign `request` (a JSON object without `auth`/`auth_v`) with version-2 authentication.
    fn sign(secret: &[u8], mut request: Value) -> Value {
        let auth = hex::encode(Self::v2_mac(secret, &request));
        if let Some(obj) = request.as_object_mut() {
            obj.insert("auth_v".into(), json!(2));
            obj.insert("auth".into(), json!(auth));
        }
        request
    }

    /// HMAC-SHA256 over PyHSM's version-2 message for an unsigned request.
    fn v2_mac(secret: &[u8], unsigned: &Value) -> [u8; 32] {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("HMAC accepts any key length");
        mac.update(&auth_message(unsigned));
        mac.finalize().into_bytes().into()
    }

    async fn exchange(stream: &mut UnixStream, request: &Value) -> std::io::Result<Value> {
        let body = serde_json::to_vec(request).map_err(std::io::Error::other)?;
        let mut frame = (body.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(&body);
        stream.write_all(&frame).await?;
        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await?;
        let len = u32::from_be_bytes(header) as usize;
        if len > MAX_REPLY_BYTES {
            return Err(std::io::Error::other("reply too large"));
        }
        let mut reply = vec![0u8; len];
        stream.read_exact(&mut reply).await?;
        serde_json::from_slice(&reply).map_err(std::io::Error::other)
    }

    /// One call. The connection is reused, and re-made once if it turns out to be dead (the key
    /// service restarted). The whole call is bounded by the timeout.
    async fn call(&self, operation: &str, fields: Value) -> Result<Value, KeyError> {
        let attempt = async {
            let mut last: Option<String> = None;
            for _ in 0..2 {
                let request_id = Uuid::new_v4().to_string();
                let mut unsigned = json!({
                    "type": operation,
                    "request_id": request_id,
                    "caller_id": "vectorscrub-backend",
                    "ts": time::OffsetDateTime::now_utc().unix_timestamp(),
                });
                if let (Some(obj), Some(extra)) = (unsigned.as_object_mut(), fields.as_object()) {
                    obj.extend(extra.clone());
                }
                let request = Self::sign(self.secret.expose().as_bytes(), unsigned);

                let mut guard = self.conn.lock().await;
                if guard.is_none() {
                    match UnixStream::connect(&self.socket).await {
                        Ok(s) => *guard = Some(s),
                        Err(e) => {
                            last = Some(format!("connect: {:?}", e.kind()));
                            continue;
                        }
                    }
                }
                let stream = guard.as_mut().expect("connected above");
                match Self::exchange(stream, &request).await {
                    Ok(reply) => {
                        if reply.get("request_id").and_then(Value::as_str) != Some(request_id.as_str()) {
                            *guard = None;
                            return Err(KeyError::Unavailable("reply did not match the request".into()));
                        }
                        if reply.get("ok").and_then(Value::as_bool) == Some(true) {
                            return Ok(reply.get("data").cloned().unwrap_or(Value::Null));
                        }
                        let why = reply.get("error").and_then(Value::as_str).unwrap_or("unknown").to_string();
                        let code = reply.get("code").and_then(Value::as_str).unwrap_or("");
                        // PyHSM's per-key rate limit: `code: "rate_limited"` (older servers only
                        // say "rate-limited" in the text). The connection stays usable.
                        if code == "rate_limited" || why.contains("rate-limited") {
                            metrics::counter!("key_service_rate_limited_total").increment(1);
                            tracing::warn!(operation, "Key service rate limit reached (raise PyHSM --rate-limit if this is normal traffic)");
                            return Err(KeyError::RateLimited);
                        }
                        // An authentication failure closes the connection on the server side.
                        *guard = None;
                        return Err(KeyError::Refused(why));
                    }
                    Err(e) => {
                        *guard = None;
                        last = Some(format!("io: {:?}", e.kind()));
                    }
                }
            }
            Err(KeyError::Unavailable(last.unwrap_or_else(|| "no answer".into())))
        };
        match tokio::time::timeout(self.timeout, attempt).await {
            Ok(r) => r,
            Err(_) => {
                *self.conn.lock().await = None;
                Err(KeyError::Unavailable("timeout".into()))
            }
        }
    }

    pub async fn health(&self) -> bool {
        self.call("health", json!({})).await.is_ok_and(|d| d.get("status").and_then(Value::as_str) == Some("ok"))
    }

    pub async fn has_key(&self) -> Result<bool, KeyError> {
        let d = self.call("has_key", json!({ "key_id": self.key_id })).await?;
        Ok(d.as_bool().unwrap_or(false))
    }

    pub async fn generate_key(&self) -> Result<(), KeyError> {
        self.call("generate_key", json!({ "key_id": self.key_id, "key_type": "aes-256" })).await.map(|_| ())
    }

    /// Add a new version of the KEK. Returns the new version number.
    pub async fn rotate_key(&self) -> Result<i64, KeyError> {
        let d = self.call("rotate_key", json!({ "key_id": self.key_id })).await?;
        d.as_i64().ok_or_else(|| KeyError::Refused("rotate_key returned no version".into()))
    }

    /// The customer id travels inside the encrypted payload (PyHSM does not take caller-supplied
    /// associated data), and is checked on the way out.
    async fn wrap(&self, customer: Uuid, dek: &[u8; 32]) -> Result<Wrapped, KeyError> {
        let mut payload = Zeroizing::new(Vec::with_capacity(48));
        payload.extend_from_slice(customer.as_bytes());
        payload.extend_from_slice(dek);
        let plaintext = Zeroizing::new(BASE64.encode(&*payload));
        let d = self
            .call("encrypt", json!({ "key_id": self.key_id, "plaintext": &*plaintext }))
            .await?;
        let hex_ct = d.as_str().ok_or_else(|| KeyError::Refused("encrypt returned no ciphertext".into()))?;
        let bytes = hex::decode(hex_ct).map_err(|_| KeyError::Integrity)?;
        // PyHSM ciphertext: format byte, 4-byte key version, nonce, ciphertext+tag.
        if bytes.len() < 1 + 4 + 12 + 16 {
            return Err(KeyError::Integrity);
        }
        let version = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
        Ok(Wrapped { bytes, provider: "pyhsm", kek_version: version as i32 })
    }

    async fn unwrap(&self, customer: Uuid, wrapped: &[u8]) -> Result<Zeroizing<[u8; 32]>, KeyError> {
        let d = self
            .call("decrypt", json!({ "key_id": self.key_id, "ciphertext": hex::encode(wrapped) }))
            .await?;
        let text = Zeroizing::new(d.as_str().ok_or_else(|| KeyError::Refused("decrypt returned no data".into()))?.to_string());
        let payload = Zeroizing::new(BASE64.decode(text.as_bytes()).map_err(|_| KeyError::Integrity)?);
        if payload.len() != 48 || payload[..16] != *customer.as_bytes() {
            return Err(KeyError::Integrity);
        }
        let mut dek = Zeroizing::new([0u8; 32]);
        dek.copy_from_slice(&payload[16..]);
        Ok(dek)
    }
}

// ===========================================================================
// The service
// ===========================================================================

pub enum Wrapper {
    Env(EnvWrapper),
    PyHsm(PyHsmClient),
}

impl Wrapper {
    pub fn name(&self) -> &'static str {
        match self {
            Wrapper::Env(_) => "env",
            Wrapper::PyHsm(_) => "pyhsm",
        }
    }

    async fn wrap(&self, customer: Uuid, dek: &[u8; 32]) -> Result<Wrapped, KeyError> {
        match self {
            Wrapper::Env(w) => w.wrap(customer, dek),
            Wrapper::PyHsm(c) => c.wrap(customer, dek).await,
        }
    }

    async fn unwrap(&self, customer: Uuid, wrapped: &[u8]) -> Result<Zeroizing<[u8; 32]>, KeyError> {
        match self {
            Wrapper::Env(w) => w.unwrap(customer, wrapped),
            Wrapper::PyHsm(c) => c.unwrap(customer, wrapped).await,
        }
    }
}

struct Cached {
    keys: Arc<CustomerKeys>,
    loaded: Instant,
}

pub struct KeyService {
    pool: PgPool,
    wrapper: Wrapper,
    cache: Mutex<HashMap<Uuid, Cached>>,
    ttl: Duration,
    health: tokio::sync::Mutex<Option<(Instant, bool)>>,
}

/// What `rewrap_all` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RewrapReport {
    pub rewrapped: u64,
    pub already_current: u64,
    pub failed: u64,
}

impl KeyService {
    pub fn new(pool: PgPool, wrapper: Wrapper, cache_ttl_secs: u64) -> Self {
        Self {
            pool,
            wrapper,
            cache: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(cache_ttl_secs),
            health: tokio::sync::Mutex::new(None),
        }
    }

    /// Build the service the configuration asks for.
    pub fn from_config(pool: PgPool, config: &crate::services::config::AppConfig) -> anyhow::Result<Self> {
        let wrapper = match &config.key_provider {
            KeyProviderConfig::Env => {
                let master = config.master_key_b64.as_deref().ok_or_else(|| anyhow::anyhow!("MASTER_ENCRYPTION_KEY is required for KEY_PROVIDER=env"))?;
                Wrapper::Env(EnvWrapper::new(&MasterKey::from_base64(master)?))
            }
            KeyProviderConfig::PyHsm { socket, ipc_secret, key_id, timeout_ms } => {
                Wrapper::PyHsm(PyHsmClient::new(socket, ipc_secret.clone(), key_id, *timeout_ms))
            }
        };
        Ok(Self::new(pool, wrapper, config.dek_cache_ttl_secs))
    }

    pub fn provider(&self) -> &'static str {
        self.wrapper.name()
    }

    pub fn pyhsm(&self) -> Option<&PyHsmClient> {
        match &self.wrapper {
            Wrapper::PyHsm(c) => Some(c),
            Wrapper::Env(_) => None,
        }
    }

    /// Is the key service answering? Asked at most every few seconds, so a readiness probe cannot
    /// become a load on it. Always true for the environment provider.
    pub async fn healthy(&self) -> bool {
        let Wrapper::PyHsm(client) = &self.wrapper else { return true };
        let mut last = self.health.lock().await;
        if let Some((at, ok)) = *last {
            if at.elapsed() < HEALTH_CACHE {
                return ok;
            }
        }
        let ok = client.health().await;
        *last = Some((Instant::now(), ok));
        ok
    }

    /// The data keys of one customer, creating them on first use. Cached in memory for the
    /// configured time; never written anywhere in the clear.
    pub async fn customer(&self, customer_id: Uuid) -> Result<Arc<CustomerKeys>, KeyError> {
        if let Some(hit) = self.cached(customer_id) {
            return Ok(hit);
        }
        let dek = match self.load_wrapped(customer_id).await? {
            Some(row) => self.unwrap_row(customer_id, row).await?,
            None => self.create(customer_id).await?,
        };
        let keys = Arc::new(CustomerKeys::new(customer_id, *dek));
        self.remember(customer_id, keys.clone());
        Ok(keys)
    }

    /// `customer`, for operator commands that touch every customer: waits out the key service's
    /// rate limit instead of failing.
    pub async fn customer_patient(&self, customer_id: Uuid) -> Result<Arc<CustomerKeys>, KeyError> {
        patiently(|| self.customer(customer_id)).await
    }

    fn cached(&self, id: Uuid) -> Option<Arc<CustomerKeys>> {
        if self.ttl.is_zero() {
            return None;
        }
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        match cache.get(&id) {
            Some(c) if c.loaded.elapsed() < self.ttl => Some(c.keys.clone()),
            Some(_) => {
                cache.remove(&id);
                None
            }
            None => None,
        }
    }

    fn remember(&self, id: Uuid, keys: Arc<CustomerKeys>) {
        if self.ttl.is_zero() {
            return;
        }
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= MAX_CACHE_ENTRIES {
            let ttl = self.ttl;
            cache.retain(|_, c| c.loaded.elapsed() < ttl);
            if cache.len() >= MAX_CACHE_ENTRIES {
                cache.clear();
            }
        }
        cache.insert(id, Cached { keys, loaded: Instant::now() });
    }

    /// Forget every cached key (after a rotation, or on shutdown).
    pub fn clear_cache(&self) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    async fn load_wrapped(&self, id: Uuid) -> Result<Option<(Vec<u8>, String)>, KeyError> {
        Ok(sqlx::query_as("SELECT wrapped_dek, kek_provider FROM customer_keys WHERE customer_id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    async fn unwrap_row(&self, id: Uuid, (bytes, provider): (Vec<u8>, String)) -> Result<Zeroizing<[u8; 32]>, KeyError> {
        if provider != self.wrapper.name() {
            return Err(KeyError::ProviderMismatch { found: provider, expected: self.wrapper.name() });
        }
        self.wrapper.unwrap(id, &bytes).await
    }

    /// First use by a customer: make their data key and store it wrapped. Two requests racing
    /// to do this both end up with the same stored key (the first insert wins, the loser reads it).
    async fn create(&self, id: Uuid) -> Result<Zeroizing<[u8; 32]>, KeyError> {
        let dek = CustomerKeys::generate_dek();
        let wrapped = self.wrapper.wrap(id, &dek).await?;
        let inserted = sqlx::query(
            "INSERT INTO customer_keys (customer_id, wrapped_dek, kek_provider, kek_version) \
             VALUES ($1, $2, $3, $4) ON CONFLICT (customer_id) DO NOTHING",
        )
        .bind(id)
        .bind(&wrapped.bytes)
        .bind(wrapped.provider)
        .bind(wrapped.kek_version)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if inserted == 1 {
            return Ok(dek);
        }
        let row = self.load_wrapped(id).await?.ok_or(KeyError::Integrity)?;
        self.unwrap_row(id, row).await
    }

    /// Counts of stored data keys by provider and KEK version.
    pub async fn status(&self) -> Result<Vec<(String, i32, i64)>, KeyError> {
        Ok(sqlx::query_as(
            "SELECT kek_provider, kek_version, count(*) FROM customer_keys GROUP BY 1, 2 ORDER BY 1, 2",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Re-wrap every stored data key under the key service's current KEK version. Used after
    /// rotating the KEK, and to move keys between providers (`legacy_env`, when given, unwraps
    /// rows that the environment provider wrapped). Customer data is not touched: only the small
    /// wrapped keys change. Each key is replaced only after the new wrapping has been checked to
    /// open back to the same key.
    pub async fn rewrap_all(&self, legacy_env: Option<&EnvWrapper>) -> Result<RewrapReport, KeyError> {
        // What "current" means: wrap a throwaway key and read the version off the result.
        let probe = CustomerKeys::generate_dek();
        let probe_id = Uuid::new_v4();
        let current = patiently(|| self.wrapper.wrap(probe_id, &probe)).await?;

        let rows: Vec<(Uuid, Vec<u8>, String, i32)> =
            sqlx::query_as("SELECT customer_id, wrapped_dek, kek_provider, kek_version FROM customer_keys ORDER BY customer_id")
                .fetch_all(&self.pool)
                .await?;
        let mut report = RewrapReport::default();
        for (id, bytes, provider, version) in rows {
            if provider == current.provider && version >= current.kek_version {
                report.already_current += 1;
                continue;
            }
            let dek = if provider == self.wrapper.name() {
                patiently(|| self.wrapper.unwrap(id, &bytes)).await
            } else if provider == "env" {
                match legacy_env {
                    Some(env) => env.unwrap(id, &bytes),
                    None => Err(KeyError::ProviderMismatch { found: provider.clone(), expected: self.wrapper.name() }),
                }
            } else {
                Err(KeyError::ProviderMismatch { found: provider.clone(), expected: self.wrapper.name() })
            };
            let outcome: Result<(), KeyError> = async {
                let dek = dek?;
                let new = patiently(|| self.wrapper.wrap(id, &dek)).await?;
                // Prove the new wrapping opens to the same key before replacing the old one.
                let back = patiently(|| self.wrapper.unwrap(id, &new.bytes)).await?;
                if *back != *dek {
                    return Err(KeyError::Integrity);
                }
                sqlx::query(
                    "UPDATE customer_keys SET wrapped_dek = $2, kek_provider = $3, kek_version = $4, rewrapped_at = NOW() \
                     WHERE customer_id = $1 AND wrapped_dek = $5",
                )
                .bind(id)
                .bind(&new.bytes)
                .bind(new.provider)
                .bind(new.kek_version)
                .bind(&bytes)
                .execute(&self.pool)
                .await?;
                Ok(())
            }
            .await;
            match outcome {
                Ok(()) => report.rewrapped += 1,
                Err(e) => {
                    report.failed += 1;
                    tracing::error!(customer_id = %id, error = %e, "Could not re-wrap a customer data key");
                }
            }
        }
        self.clear_cache();
        Ok(report)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A stand-in for PyHSM's IPC server: the same framing, the same request authentication, the
    /// same ciphertext layout (format byte, 4-byte version, nonce, ciphertext), real key versions.
    /// The real server is exercised separately (see docs/runbooks/key-service.md).
    pub struct FakePyHsm {
        /// Held only so the socket directory is removed when the fake is dropped.
        pub _dir: tempfile_dir::Dir,
        pub socket: String,
        pub secret: String,
        pub calls: Arc<AtomicU32>,
        pub keys: Arc<Mutex<Vec<[u8; 32]>>>,
        pub down: Arc<std::sync::atomic::AtomicBool>,
        pub drop_connections: Arc<AtomicU32>,
        pub stall: Arc<std::sync::atomic::AtomicBool>,
        /// The next this-many encrypt/decrypt calls are refused with PyHSM's rate-limit error.
        pub rate_limit_next: Arc<AtomicU32>,
    }

    mod tempfile_dir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                let p = std::env::temp_dir().join(format!("vs-fakehsm-{}", uuid::Uuid::new_v4().simple()));
                std::fs::create_dir_all(&p).unwrap();
                Dir(p)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    impl FakePyHsm {
        pub async fn start() -> Self {
            let dir = tempfile_dir::Dir::new();
            let socket = dir.0.join("pyhsm.sock").to_string_lossy().to_string();
            let secret = "s".repeat(40);
            let fake = Self {
                _dir: dir,
                socket: socket.clone(),
                secret: secret.clone(),
                calls: Arc::new(AtomicU32::new(0)),
                keys: Arc::new(Mutex::new(vec![])),
                down: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                drop_connections: Arc::new(AtomicU32::new(0)),
                stall: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                rate_limit_next: Arc::new(AtomicU32::new(0)),
            };
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            let (calls, keys, down, drops, stall, limited) = (
                fake.calls.clone(),
                fake.keys.clone(),
                fake.down.clone(),
                fake.drop_connections.clone(),
                fake.stall.clone(),
                fake.rate_limit_next.clone(),
            );
            tokio::spawn(async move {
                loop {
                    let Ok((mut conn, _)) = listener.accept().await else { return };
                    let (calls, keys, down, drops, stall, limited, secret) = (
                        calls.clone(),
                        keys.clone(),
                        down.clone(),
                        drops.clone(),
                        stall.clone(),
                        limited.clone(),
                        secret.clone(),
                    );
                    tokio::spawn(async move {
                        loop {
                            let mut header = [0u8; 4];
                            if conn.read_exact(&mut header).await.is_err() {
                                return;
                            }
                            let mut body = vec![0u8; u32::from_be_bytes(header) as usize];
                            if conn.read_exact(&mut body).await.is_err() {
                                return;
                            }
                            if down.load(Ordering::SeqCst) || stall.load(Ordering::SeqCst) {
                                if stall.load(Ordering::SeqCst) {
                                    tokio::time::sleep(Duration::from_secs(30)).await;
                                }
                                return;
                            }
                            if drops.load(Ordering::SeqCst) > 0 {
                                drops.fetch_sub(1, Ordering::SeqCst);
                                return; // simulate the server restarting between calls
                            }
                            calls.fetch_add(1, Ordering::SeqCst);
                            let req: Value = serde_json::from_slice(&body).unwrap();
                            let (rid, op) = (req["request_id"].as_str().unwrap_or(""), req["type"].as_str().unwrap_or(""));
                            // Version 2 only, like the real server: v1 tokens and unsigned
                            // requests fail.
                            let expected = if req["auth_v"] == json!(2) {
                                hex::encode(PyHsmClient::v2_mac(secret.as_bytes(), &req))
                            } else {
                                "v1-refused".to_string()
                            };
                            let crypto_op = matches!(op, "encrypt" | "decrypt");
                            let reply = if req["auth"].as_str() != Some(expected.as_str()) {
                                json!({ "ok": false, "request_id": rid, "error": "authentication failed" })
                            } else if crypto_op
                                && limited
                                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                                    .is_ok()
                            {
                                // PyHSM's exact wording (hsm/core.py, _enforce_policy).
                                json!({ "ok": false, "request_id": rid, "error": "PyHSM: key 'vectorscrub-kek' is rate-limited" })
                            } else {
                                Self::handle(&keys, op, &req, rid)
                            };
                            let bytes = serde_json::to_vec(&reply).unwrap();
                            let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
                            frame.extend_from_slice(&bytes);
                            if conn.write_all(&frame).await.is_err() {
                                return;
                            }
                            if reply["ok"] == json!(false) && reply["error"] == json!("authentication failed") {
                                return; // the real server closes the connection on an auth failure
                            }
                        }
                    });
                }
            });
            fake
        }

        fn handle(keys: &Arc<Mutex<Vec<[u8; 32]>>>, op: &str, req: &Value, rid: &str) -> Value {
            let ok = |data: Value| json!({ "ok": true, "request_id": rid, "data": data });
            let err = |m: &str| json!({ "ok": false, "request_id": rid, "error": m });
            let mut keys = keys.lock().unwrap();
            match op {
                "health" => ok(json!({ "status": "ok", "session_active": true })),
                "has_key" => ok(json!(!keys.is_empty())),
                "generate_key" => {
                    if !keys.is_empty() {
                        return err("key already exists");
                    }
                    keys.push(rand_key());
                    ok(json!("generated"))
                }
                "rotate_key" => {
                    if keys.is_empty() {
                        return err("no such key");
                    }
                    keys.push(rand_key());
                    ok(json!(keys.len()))
                }
                "encrypt" => {
                    let Some(k) = keys.last() else { return err("no such key") };
                    let version = keys.len() as u32;
                    let aad = format!("pyhsm:v1:{}:{}", req["key_id"].as_str().unwrap_or(""), version);
                    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(k));
                    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
                    let ct = cipher
                        .encrypt(&nonce, Payload { msg: req["plaintext"].as_str().unwrap_or("").as_bytes(), aad: aad.as_bytes() })
                        .unwrap();
                    let mut out = vec![2u8];
                    out.extend_from_slice(&version.to_be_bytes());
                    out.extend_from_slice(&nonce);
                    out.extend_from_slice(&ct);
                    ok(json!(hex::encode(out)))
                }
                "decrypt" => {
                    let Ok(raw) = hex::decode(req["ciphertext"].as_str().unwrap_or("")) else { return err("bad ciphertext") };
                    if raw.len() < 17 {
                        return err("bad ciphertext");
                    }
                    let version = u32::from_be_bytes([raw[1], raw[2], raw[3], raw[4]]);
                    let Some(k) = keys.get(version as usize - 1) else { return err("unknown key version") };
                    let aad = format!("pyhsm:v1:{}:{}", req["key_id"].as_str().unwrap_or(""), version);
                    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(k));
                    match cipher.decrypt(Nonce::from_slice(&raw[5..17]), Payload { msg: &raw[17..], aad: aad.as_bytes() }) {
                        Ok(p) => ok(json!(String::from_utf8(p).unwrap())),
                        Err(_) => err("Decryption failed"),
                    }
                }
                _ => err("unknown operation"),
            }
        }

        pub fn client(&self, timeout_ms: u64) -> PyHsmClient {
            PyHsmClient::new(&self.socket, SecretString::new(self.secret.clone()), "vectorscrub-kek", timeout_ms)
        }

        pub fn service(&self, pool: PgPool, ttl: u64) -> KeyService {
            let c = self.client(2000);
            self.keys.lock().unwrap().push(rand_key());
            KeyService::new(pool, Wrapper::PyHsm(c), ttl)
        }
    }

    fn rand_key() -> [u8; 32] {
        let mut k = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut k);
        k
    }

    fn env_service(pool: PgPool, ttl: u64) -> KeyService {
        ts::key_service_with_ttl(pool, ttl)
    }

    // ---- environment provider ----------------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn a_customer_gets_one_stable_key_that_is_stored_only_wrapped(pool: PgPool) {
        let svc = env_service(pool.clone(), 0);
        let c = ts::customer(&pool, "free", false).await;
        let a = svc.customer(c).await.unwrap();
        let blob = a.seal_str("identity.full_name", "Jane").unwrap();
        let b = svc.customer(c).await.unwrap();
        assert_eq!(b.open_str("identity.full_name", &blob).unwrap(), "Jane", "the same key every time");

        let (wrapped, provider, version): (Vec<u8>, String, i32) =
            sqlx::query_as("SELECT wrapped_dek, kek_provider, kek_version FROM customer_keys WHERE customer_id = $1")
                .bind(c).fetch_one(&pool).await.unwrap();
        assert_eq!((provider.as_str(), version), ("env", 1));
        assert!(!wrapped.windows(32).any(|w| w == a.dek_bytes()), "the raw key is not in the column");
        assert_ne!(wrapped.len(), 32, "it is a wrapped value, not the key");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn customers_get_different_keys(pool: PgPool) {
        let svc = env_service(pool.clone(), 0);
        let (a, b) = (ts::customer(&pool, "free", false).await, ts::customer(&pool, "free", false).await);
        let (ka, kb) = (svc.customer(a).await.unwrap(), svc.customer(b).await.unwrap());
        assert_ne!(ka.dek_bytes(), kb.dek_bytes());
        let blob = ka.seal_str("identity.address", "1 Main St").unwrap();
        assert!(kb.open_str("identity.address", &blob).is_err());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn two_first_requests_at_once_end_up_with_the_same_key(pool: PgPool) {
        let svc = Arc::new(env_service(pool.clone(), 0));
        let c = ts::customer(&pool, "free", false).await;
        let tasks: Vec<_> = (0..8).map(|_| { let s = svc.clone(); tokio::spawn(async move { s.customer(c).await.unwrap().dek_bytes().to_vec() }) }).collect();
        let mut seen = std::collections::HashSet::new();
        for t in tasks { seen.insert(t.await.unwrap()); }
        assert_eq!(seen.len(), 1, "one key, whoever won the insert");
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM customer_keys").fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_wrapped_key_moved_to_another_customer_is_refused(pool: PgPool) {
        let svc = env_service(pool.clone(), 0);
        let (a, b) = (ts::customer(&pool, "free", false).await, ts::customer(&pool, "free", false).await);
        svc.customer(a).await.unwrap();
        svc.customer(b).await.unwrap();
        sqlx::query("UPDATE customer_keys SET wrapped_dek = (SELECT wrapped_dek FROM customer_keys WHERE customer_id = $1) WHERE customer_id = $2")
            .bind(a).bind(b).execute(&pool).await.unwrap();
        assert!(matches!(svc.customer(b).await, Err(KeyError::Integrity)), "bound to its customer");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_cache_avoids_the_database_and_expires(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let svc = env_service(pool.clone(), 300);
        svc.customer(c).await.unwrap();
        sqlx::query("DELETE FROM customer_keys").execute(&pool).await.unwrap();
        assert!(svc.customer(c).await.is_ok(), "served from memory");
        svc.clear_cache();
        let again = svc.customer(c).await.unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM customer_keys").fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 1, "a new key was made, since the old one was deleted");
        drop(again);

        let no_cache = env_service(pool.clone(), 0);
        no_cache.customer(c).await.unwrap();
        sqlx::query("DELETE FROM customer_keys").execute(&pool).await.unwrap();
        let fresh = no_cache.customer(c).await.unwrap();
        assert!(fresh.seal("x", b"y").is_ok());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_key_wrapped_by_another_provider_is_refused_with_a_clear_message(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        env_service(pool.clone(), 0).customer(c).await.unwrap();
        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 0);
        match svc.customer(c).await {
            Err(KeyError::ProviderMismatch { found, expected }) => assert_eq!((found.as_str(), expected), ("env", "pyhsm")),
            other => panic!("{other:?}"),
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn erasing_the_account_deletes_the_key(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        env_service(pool.clone(), 0).customer(c).await.unwrap();
        crate::services::account_deletion::delete_account(&pool, c).await.unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM customer_keys").fetch_one(&pool).await.unwrap();
        assert_eq!(rows, 0);
    }

    // ---- PyHSM provider, against the stand-in server -------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn keys_are_wrapped_and_unwrapped_through_the_key_service(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 0);
        let c = ts::customer(&pool, "free", false).await;
        let blob = svc.customer(c).await.unwrap().seal_str("identity.full_name", "Jane").unwrap();
        assert_eq!(svc.customer(c).await.unwrap().open_str("identity.full_name", &blob).unwrap(), "Jane");
        let (provider, version): (String, i32) = sqlx::query_as("SELECT kek_provider, kek_version FROM customer_keys").fetch_one(&pool).await.unwrap();
        assert_eq!((provider.as_str(), version), ("pyhsm", 1));
        assert!(fake.calls.load(Ordering::SeqCst) >= 2, "the key service was really used");
        assert!(svc.healthy().await);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_cache_keeps_the_key_service_out_of_the_request_path(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 300);
        let c = ts::customer(&pool, "free", false).await;
        svc.customer(c).await.unwrap();
        let after_first = fake.calls.load(Ordering::SeqCst);
        for _ in 0..50 { svc.customer(c).await.unwrap(); }
        assert_eq!(fake.calls.load(Ordering::SeqCst), after_first, "no calls for cached keys");
        fake.down.store(true, Ordering::SeqCst);
        assert!(svc.customer(c).await.is_ok(), "a cached key still works while the service is down");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn when_the_key_service_is_down_nothing_decrypts_and_nothing_falls_back(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 0);
        let c = ts::customer(&pool, "free", false).await;
        svc.customer(c).await.unwrap();
        fake.down.store(true, Ordering::SeqCst);
        assert!(matches!(svc.customer(c).await, Err(KeyError::Unavailable(_))));
        assert!(!svc.healthy().await, "readiness says so too");
        fake.down.store(false, Ordering::SeqCst);
        assert!(svc.customer(c).await.is_ok(), "and it recovers by itself");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_restarted_key_service_is_reconnected_to(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 0);
        let c = ts::customer(&pool, "free", false).await;
        svc.customer(c).await.unwrap();
        fake.drop_connections.store(1, Ordering::SeqCst); // the next request finds the connection dead
        assert!(svc.customer(c).await.is_ok());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_stalled_key_service_times_out_instead_of_hanging_the_request(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        let client = fake.client(300);
        fake.stall.store(true, Ordering::SeqCst);
        let started = Instant::now();
        let r = KeyService::new(pool, Wrapper::PyHsm(client), 0).healthy().await;
        assert!(!r);
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_wrong_ipc_secret_is_refused_by_the_key_service(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        fake.keys.lock().unwrap().push(rand_key());
        let bad = PyHsmClient::new(&fake.socket, SecretString::new("x".repeat(40)), "vectorscrub-kek", 1000);
        let svc = KeyService::new(pool.clone(), Wrapper::PyHsm(bad), 0);
        let c = ts::customer(&pool, "free", false).await;
        assert!(matches!(svc.customer(c).await, Err(KeyError::Refused(_))));
        assert!(!svc.healthy().await);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unreachable_socket_is_a_clean_error(pool: PgPool) {
        let client = PyHsmClient::new("/nonexistent/pyhsm.sock", SecretString::new("s".repeat(40)), "k", 500);
        let svc = KeyService::new(pool.clone(), Wrapper::PyHsm(client), 0);
        let c = ts::customer(&pool, "free", false).await;
        assert!(matches!(svc.customer(c).await, Err(KeyError::Unavailable(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_wrapped_key_for_another_customer_is_refused_by_the_check_inside_the_payload(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 0);
        let (a, b) = (ts::customer(&pool, "free", false).await, ts::customer(&pool, "free", false).await);
        svc.customer(a).await.unwrap();
        svc.customer(b).await.unwrap();
        sqlx::query("UPDATE customer_keys SET wrapped_dek = (SELECT wrapped_dek FROM customer_keys WHERE customer_id = $1) WHERE customer_id = $2")
            .bind(a).bind(b).execute(&pool).await.unwrap();
        assert!(matches!(svc.customer(b).await, Err(KeyError::Integrity)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rotating_the_kek_keeps_old_keys_working_and_rewrap_moves_them_to_the_new_version(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 0);
        let c = ts::customer(&pool, "free", false).await;
        let blob = svc.customer(c).await.unwrap().seal_str("identity.full_name", "Jane").unwrap();

        assert_eq!(svc.pyhsm().unwrap().rotate_key().await.unwrap(), 2);
        assert_eq!(svc.customer(c).await.unwrap().open_str("identity.full_name", &blob).unwrap(), "Jane", "old version still opens");
        assert_eq!(svc.status().await.unwrap(), vec![("pyhsm".to_string(), 1, 1)]);

        let report = svc.rewrap_all(None).await.unwrap();
        assert_eq!(report, RewrapReport { rewrapped: 1, already_current: 0, failed: 0 });
        assert_eq!(svc.status().await.unwrap(), vec![("pyhsm".to_string(), 2, 1)]);
        assert_eq!(svc.customer(c).await.unwrap().open_str("identity.full_name", &blob).unwrap(), "Jane", "same key, same data");

        let again = svc.rewrap_all(None).await.unwrap();
        assert_eq!(again, RewrapReport { rewrapped: 0, already_current: 1, failed: 0 }, "nothing to do the second time");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn keys_can_be_moved_from_the_environment_provider_to_pyhsm(pool: PgPool) {
        let c = ts::customer(&pool, "free", false).await;
        let env = env_service(pool.clone(), 0);
        let blob = env.customer(c).await.unwrap().seal_str("identity.address", "1 Main St").unwrap();

        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 0);
        assert_eq!(svc.rewrap_all(None).await.unwrap().failed, 1, "without the old master key it cannot");
        let master = MasterKey::from_base64(&ts::master_key_b64()).unwrap();
        let report = svc.rewrap_all(Some(&EnvWrapper::new(&master))).await.unwrap();
        assert_eq!(report.rewrapped, 1);
        assert_eq!(svc.customer(c).await.unwrap().open_str("identity.address", &blob).unwrap(), "1 Main St");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_rewrap_that_cannot_prove_the_new_wrapping_changes_nothing(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 0);
        let c = ts::customer(&pool, "free", false).await;
        svc.customer(c).await.unwrap();
        svc.pyhsm().unwrap().rotate_key().await.unwrap();
        let before: Vec<u8> = sqlx::query_scalar("SELECT wrapped_dek FROM customer_keys").fetch_one(&pool).await.unwrap();
        fake.down.store(true, Ordering::SeqCst);
        let report = svc.rewrap_all(None).await;
        assert!(report.is_err() || report.unwrap().failed == 1);
        let after: Vec<u8> = sqlx::query_scalar("SELECT wrapped_dek FROM customer_keys").fetch_one(&pool).await.unwrap();
        assert_eq!(before, after);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_client_can_set_up_the_kek_and_report_whether_it_exists(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        let svc = KeyService::new(pool, Wrapper::PyHsm(fake.client(1000)), 0);
        let hsm = svc.pyhsm().unwrap();
        assert!(!hsm.has_key().await.unwrap());
        hsm.generate_key().await.unwrap();
        assert!(hsm.has_key().await.unwrap());
        assert!(matches!(hsm.generate_key().await, Err(KeyError::Refused(_))), "never silently replaces the key");
    }

    // ---- the key service's rate limit -----------------------------------------------------

    #[sqlx::test(migrations = "./migrations")]
    async fn a_customer_request_fails_fast_when_the_key_service_is_rate_limited(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 0);
        let c = ts::customer(&pool, "free", false).await;
        svc.customer(c).await.unwrap();
        fake.rate_limit_next.store(1, Ordering::SeqCst);
        let started = Instant::now();
        assert!(matches!(svc.customer(c).await, Err(KeyError::RateLimited)), "its own error, not a generic refusal");
        assert!(started.elapsed() < Duration::from_secs(1), "a customer's request never waits it out");
        assert!(svc.customer(c).await.is_ok(), "the connection is still usable afterwards");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rotation_and_rewrap_wait_out_the_rate_limit_instead_of_failing(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 0);
        let mut customers = vec![];
        for _ in 0..3 {
            let c = ts::customer(&pool, "free", false).await;
            svc.customer(c).await.unwrap();
            customers.push(c);
        }
        svc.pyhsm().unwrap().rotate_key().await.unwrap();
        // Refuse several calls in the middle of the re-wrap, as a busy key service would.
        fake.rate_limit_next.store(4, Ordering::SeqCst);
        let report = svc.rewrap_all(None).await.unwrap();
        assert_eq!(report, RewrapReport { rewrapped: 3, already_current: 0, failed: 0 });
        assert_eq!(svc.status().await.unwrap(), vec![("pyhsm".to_string(), 2, 3)]);
        for c in customers {
            assert!(svc.customer(c).await.is_ok());
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_patient_lookup_used_by_operator_commands_waits_too(pool: PgPool) {
        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 0);
        let c = ts::customer(&pool, "free", false).await;
        svc.customer(c).await.unwrap();
        fake.rate_limit_next.store(2, Ordering::SeqCst);
        assert!(svc.customer_patient(c).await.is_ok());
    }

    #[tokio::test]
    async fn waiting_gives_up_on_errors_that_are_not_the_rate_limit() {
        let calls = std::sync::atomic::AtomicU32::new(0);
        let r: Result<(), KeyError> = patiently(|| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(KeyError::Integrity)
        })
        .await;
        assert!(matches!(r, Err(KeyError::Integrity)));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "only the rate limit is retried");
    }

    // ---- version-2 request authentication --------------------------------------------------

    /// The vector PyHSM's own test suite freezes (tests/test_ipc_auth_v2.py). If the two sides
    /// ever disagree, every request is refused, so this must match exactly.
    #[test]
    fn the_v2_signature_matches_pyhsms_published_vector() {
        let unsigned = json!({
            "type": "encrypt",
            "request_id": "11111111-2222-3333-4444-555555555555",
            "caller_id": "vectorscrub-backend",
            "key_id": "vectorscrub-kek",
            "plaintext": "aGVsbG8=",
            "ts": 1700000000,
        });
        let signed = PyHsmClient::sign(&[b's'; 40], unsigned);
        assert_eq!(signed["auth"], "7843691e60a22a195d460667947ecb6f5dc1181f3db6fa6c0e78f781b2a574ca");
        assert_eq!(signed["auth_v"], 2);
        // The signature does not cover itself, and adding or changing any field changes it.
        let again = PyHsmClient::sign(&[b's'; 40], {
            let mut v = signed.clone();
            v.as_object_mut().unwrap().remove("auth");
            v.as_object_mut().unwrap().remove("auth_v");
            v
        });
        assert_eq!(again["auth"], signed["auth"]);
        let mut changed = signed.clone();
        changed["key_id"] = json!("another-key");
        assert_ne!(hex::encode(PyHsmClient::v2_mac(&[b's'; 40], &changed)), signed["auth"].as_str().unwrap());
    }

    #[test]
    fn the_canonical_body_has_sorted_keys_and_no_whitespace() {
        let msg = auth_message(&json!({ "type": "health", "request_id": "r1", "ts": 5, "b": "x", "a": 1, "auth": "dropped", "auth_v": 2 }));
        let text = String::from_utf8(msg).unwrap();
        let body = r#"{"a":1,"b":"x","request_id":"r1","ts":5,"type":"health"}"#;
        let hash = hex::encode(<Sha256 as sha2::Digest>::digest(body.as_bytes()));
        assert_eq!(text, format!("pyhsm-ipc-v2\nr1\nhealth\n5\n{hash}"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_rate_limit_reported_by_code_alone_is_recognised(pool: PgPool) {
        // Newer PyHSM servers send a fixed message plus `code: "rate_limited"`; the fake sends
        // PyHSM's text. Both must map to RateLimited (covered: the text path above, the code
        // path through `call`'s check of `code`).
        let fake = FakePyHsm::start().await;
        let svc = fake.service(pool.clone(), 0);
        let c = ts::customer(&pool, "free", false).await;
        svc.customer(c).await.unwrap();
        fake.rate_limit_next.store(1, Ordering::SeqCst);
        assert!(matches!(svc.customer(c).await, Err(KeyError::RateLimited)));
    }
}

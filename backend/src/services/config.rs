use anyhow::{Context, Result};
use std::env;

/// Application configuration loaded from environment variables.
/// Panics at startup if required variables are missing — fail fast is intentional.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub redis_url: String,
    pub jwt_secret: String,
    pub master_key_b64: String,
    pub agent_api_key: String,
    pub port: u16,
    pub db_pool_size: u32,
    pub agent_service_url: String,
    pub environment: Environment,
    pub allowed_origins: Vec<String>,
    pub otlp_endpoint: Option<String>,
    /// Trust X-Forwarded-For / X-Real-IP / Forwarded to identify the client IP for rate
    /// limiting. ONLY enable behind a reverse proxy that sets (and strips client-supplied)
    /// these headers; otherwise any client can spoof its IP and bypass the limits.
    pub trust_proxy_headers: bool,
    /// General API limit per client IP: sustained requests/second and burst capacity.
    pub rate_limit_general_per_second: u64,
    pub rate_limit_general_burst: u32,
    /// Login/register limit per client IP: sustained attempts/minute and burst capacity.
    pub rate_limit_auth_per_minute: u64,
    pub rate_limit_auth_burst: u32,
    /// Scan job recovery (see jobs/recovery.rs).
    pub scan_recovery_interval_secs: u64,
    pub scan_stale_after_secs: i64,
    pub scan_redispatch_after_secs: i64,
    pub scan_max_attempts: i32,
}

/// Secrets shorter than this are rejected at startup. 32 bytes matches the strength of
/// the keys they protect (HS256 JWT signing, the shared agent key).
pub const MIN_SECRET_LEN: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub enum Environment {
    Development,
    Staging,
    Production,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|key| env::var(key).ok())
    }

    /// Build the config from any key lookup. `from_env` passes the process environment;
    /// tests pass a map, so they never touch process-global state.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let required = |key: &str| -> Result<String> {
            get(key).with_context(|| format!("Required environment variable '{key}' is not set"))
        };
        fn env_parse<T: std::str::FromStr>(
            get: &impl Fn(&str) -> Option<String>,
            key: &str,
            default: T,
        ) -> Result<T> {
            match get(key) {
                Some(v) => v
                    .trim()
                    .parse::<T>()
                    .map_err(|_| anyhow::anyhow!("{key} has an invalid value")),
                None => Ok(default),
            }
        }

        let allowed_origins = get("ALLOWED_ORIGINS")
            .unwrap_or_else(|| "http://localhost:3000".into())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let jwt_secret = required("JWT_SECRET")?;
        let master_key_b64 = required("MASTER_ENCRYPTION_KEY")?;
        let agent_api_key = required("AGENT_API_KEY")?;

        // Fail fast on weak or malformed secrets. Messages name the variable, never the value.
        anyhow::ensure!(
            jwt_secret.len() >= MIN_SECRET_LEN,
            "JWT_SECRET must be at least {MIN_SECRET_LEN} characters (generate one with infra/scripts/gen_keys.sh)"
        );
        anyhow::ensure!(
            agent_api_key.len() >= MIN_SECRET_LEN,
            "AGENT_API_KEY must be at least {MIN_SECRET_LEN} characters (generate one with infra/scripts/gen_keys.sh)"
        );
        crate::crypto::MasterKey::from_base64(&master_key_b64)
            .context("MASTER_ENCRYPTION_KEY must be 32 bytes, base64-encoded")?;

        Ok(Self {
            database_url: required("DATABASE_URL")?,
            redis_url: get("REDIS_URL").unwrap_or_else(|| "redis://127.0.0.1:6379".into()),
            jwt_secret,
            master_key_b64,
            agent_api_key,
            port: get("PORT")
                .unwrap_or_else(|| "8080".into())
                .parse()
                .context("PORT must be a valid port number")?,
            db_pool_size: get("DB_POOL_SIZE")
                .unwrap_or_else(|| "10".into())
                .parse()
                .context("DB_POOL_SIZE must be a number")?,
            agent_service_url: get("AGENT_SERVICE_URL")
                .unwrap_or_else(|| "http://127.0.0.1:8000".into()),
            environment: match get("ENVIRONMENT").as_deref() {
                Some("production") => Environment::Production,
                Some("staging") => Environment::Staging,
                _ => Environment::Development,
            },
            allowed_origins,
            otlp_endpoint: get("OTLP_ENDPOINT"),
            trust_proxy_headers: env_parse(&get, "TRUST_PROXY_HEADERS", false)?,
            rate_limit_general_per_second: env_parse(&get, "RATE_LIMIT_GENERAL_PER_SECOND", 20)?,
            rate_limit_general_burst: env_parse(&get, "RATE_LIMIT_GENERAL_BURST", 60)?,
            rate_limit_auth_per_minute: env_parse(&get, "RATE_LIMIT_AUTH_PER_MINUTE", 5)?,
            rate_limit_auth_burst: env_parse(&get, "RATE_LIMIT_AUTH_BURST", 5)?,
            scan_recovery_interval_secs: env_parse(&get, "SCAN_RECOVERY_INTERVAL_SECS", 60)?,
            scan_stale_after_secs: env_parse(&get, "SCAN_STALE_AFTER_SECS", 600)?,
            scan_redispatch_after_secs: env_parse(&get, "SCAN_REDISPATCH_AFTER_SECS", 600)?,
            scan_max_attempts: env_parse(&get, "SCAN_MAX_ATTEMPTS", 3)?,
        })
    }

    pub fn is_production(&self) -> bool {
        self.environment == Environment::Production
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    use std::collections::HashMap;

    fn good() -> HashMap<&'static str, String> {
        HashMap::from([
            ("DATABASE_URL", "postgres://x".to_string()),
            ("JWT_SECRET", "j".repeat(40)),
            ("AGENT_API_KEY", "a".repeat(40)),
            ("MASTER_ENCRYPTION_KEY", B64.encode([5u8; 32])),
        ])
    }
    fn load(vars: &HashMap<&'static str, String>) -> Result<AppConfig> {
        AppConfig::from_lookup(|k| vars.get(k).cloned())
    }

    #[test]
    fn a_complete_environment_loads_with_safe_defaults() {
        let c = load(&good()).unwrap();
        assert_eq!(c.port, 8080);
        assert!(!c.trust_proxy_headers, "proxy headers are not trusted unless asked");
        assert_eq!((c.rate_limit_auth_per_minute, c.rate_limit_auth_burst), (5, 5));
        assert_eq!(c.scan_max_attempts, 3);
        assert!(!c.is_production());
    }

    #[test]
    fn required_variables_are_required() {
        for key in ["DATABASE_URL", "JWT_SECRET", "AGENT_API_KEY", "MASTER_ENCRYPTION_KEY"] {
            let mut v = good();
            v.remove(key);
            let err = load(&v).unwrap_err().to_string();
            assert!(err.contains(key), "{key}: {err}");
        }
    }

    #[test]
    fn weak_secrets_are_refused_without_echoing_them() {
        for key in ["JWT_SECRET", "AGENT_API_KEY"] {
            let mut v = good();
            v.insert(key, "short-secret-value".to_string());
            let err = load(&v).unwrap_err().to_string();
            assert!(err.contains(key) && err.contains("32"), "{err}");
            assert!(!err.contains("short-secret-value"), "the secret must never appear in the error");
        }
        let mut v = good();
        v.insert("AGENT_API_KEY", "a".repeat(MIN_SECRET_LEN)); // exactly the minimum is fine
        assert!(load(&v).is_ok());
        v.insert("AGENT_API_KEY", "a".repeat(MIN_SECRET_LEN - 1));
        assert!(load(&v).is_err());
    }

    #[test]
    fn a_malformed_master_key_fails_at_startup_not_on_the_first_request() {
        for bad in ["not base64!!", "", &B64.encode([1u8; 16]), &B64.encode([1u8; 64])] {
            let mut v = good();
            v.insert("MASTER_ENCRYPTION_KEY", bad.to_string());
            let err = load(&v).unwrap_err().to_string();
            assert!(err.contains("MASTER_ENCRYPTION_KEY"), "{err}");
            assert!(!err.contains(bad) || bad.is_empty());
        }
    }

    #[test]
    fn overrides_are_parsed_and_bad_numbers_are_rejected() {
        let mut v = good();
        v.insert("TRUST_PROXY_HEADERS", "true".into());
        v.insert("RATE_LIMIT_AUTH_PER_MINUTE", " 12 ".into());
        v.insert("ENVIRONMENT", "production".into());
        let c = load(&v).unwrap();
        assert!(c.trust_proxy_headers && c.is_production());
        assert_eq!(c.rate_limit_auth_per_minute, 12);

        v.insert("SCAN_MAX_ATTEMPTS", "three".into());
        let err = load(&v).unwrap_err().to_string();
        assert!(err.contains("SCAN_MAX_ATTEMPTS"), "{err}");
    }
}

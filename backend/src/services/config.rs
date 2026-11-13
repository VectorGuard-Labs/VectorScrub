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
    /// How many of OUR proxies sit in front of this service when `trust_proxy_headers` is on.
    /// The client address is the `X-Forwarded-For` entry this many places from the right (the
    /// left side is client-controlled). 1 for a single nginx / load balancer.
    pub trusted_proxy_hops: usize,
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
    /// Stripe billing. `None` (no `STRIPE_WEBHOOK_SECRET`) leaves billing switched off: the
    /// webhook answers 404 and no plan is ever changed by it.
    pub stripe: Option<StripeConfig>,
    /// Deletion requests sent to companies on the customer's behalf. OFF unless this is set:
    /// the endpoints answer 404 and nothing is queued. The authorization wording needs a privacy
    /// lawyer's review before it is turned on for real customers.
    pub deletion_requests_enabled: bool,
    /// Stop accepting new accounts once this many exist (`MAX_ACCOUNTS`). Unset means no limit.
    /// Used to keep a pilot small. The check is not atomic, so a burst of simultaneous
    /// registrations can overshoot by a few.
    pub max_accounts: Option<i64>,
}

/// A paid plan. Ordered, so the highest of a customer's subscriptions is `max()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PaidTier {
    Personal,
    Premium,
}

impl PaidTier {
    pub fn as_str(self) -> &'static str {
        match self {
            PaidTier::Personal => "personal",
            PaidTier::Premium => "premium",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingInterval {
    Month,
    Year,
}

impl BillingInterval {
    pub fn as_str(self) -> &'static str {
        match self {
            BillingInterval::Month => "month",
            BillingInterval::Year => "year",
        }
    }
}

/// One purchasable plan: which Stripe price it is, and the Stripe Payment Link that sells it.
#[derive(Debug, Clone)]
pub struct StripePlan {
    pub tier: PaidTier,
    pub interval: BillingInterval,
    /// The price currently sold for this plan (the first id listed in `STRIPE_PRICE_*`).
    pub price_id: String,
    /// Earlier prices of this plan, still honoured for people who subscribed at them (the
    /// ids after the first). This is the "price lock": raising the price creates a new Stripe
    /// price, and existing members stay on the old one and keep their plan.
    pub legacy_price_ids: Vec<String>,
    /// The Payment Link selling `price_id`.
    pub link: Option<String>,
}

impl StripePlan {
    /// Whether a subscription at this Stripe price entitles the customer to this plan.
    pub fn sells(&self, price_id: &str) -> bool {
        self.price_id == price_id || self.legacy_price_ids.iter().any(|p| p == price_id)
    }
}

#[derive(Debug, Clone)]
pub struct StripeConfig {
    /// Signing secret of the webhook endpoint (`whsec_...`). Verifies that a webhook really
    /// came from Stripe, and keys the signature on the customer reference in checkout links.
    pub webhook_secret: String,
    pub plans: Vec<StripePlan>,
    /// Stripe Customer Portal login link, where customers cancel, switch plan or change card.
    pub portal_url: Option<String>,
}

impl StripeConfig {
    /// Read the Stripe settings. Messages name the variable, never its value.
    fn from_lookup(get: &impl Fn(&str) -> Option<String>) -> Result<Option<Self>> {
        let nonblank = |key: &str| get(key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let Some(webhook_secret) = nonblank("STRIPE_WEBHOOK_SECRET") else {
            return Ok(None);
        };
        anyhow::ensure!(
            webhook_secret.len() >= 16,
            "STRIPE_WEBHOOK_SECRET looks too short to be a Stripe signing secret (whsec_...)"
        );

        let mut plans: Vec<StripePlan> = Vec::new();
        for (tier, tier_name) in [(PaidTier::Personal, "PERSONAL"), (PaidTier::Premium, "PREMIUM")] {
            for (interval, interval_name) in
                [(BillingInterval::Month, "MONTHLY"), (BillingInterval::Year, "ANNUAL")]
            {
                let price_key = format!("STRIPE_PRICE_{tier_name}_{interval_name}");
                let link_key = format!("STRIPE_LINK_{tier_name}_{interval_name}");
                let Some(raw_prices) = nonblank(&price_key) else { continue };
                // "price_current" or "price_current,price_older,...": see StripePlan.
                let mut ids = raw_prices.split(',').map(str::trim).filter(|v| !v.is_empty());
                let Some(price_id) = ids.next().map(String::from) else { continue };
                let legacy_price_ids: Vec<String> = ids.map(String::from).collect();
                for id in std::iter::once(&price_id).chain(&legacy_price_ids) {
                    anyhow::ensure!(
                        id.starts_with("price_"),
                        "{price_key} must list Stripe price ids (price_...), separated by commas"
                    );
                    anyhow::ensure!(
                        !plans.iter().any(|p| p.sells(id)),
                        "{price_key} repeats a price id already used by another plan"
                    );
                }
                anyhow::ensure!(
                    !legacy_price_ids.contains(&price_id)
                        && legacy_price_ids.iter().enumerate().all(|(i, id)| !legacy_price_ids[..i].contains(id)),
                    "{price_key} lists the same price id twice"
                );
                let link = nonblank(&link_key);
                if let Some(l) = &link {
                    anyhow::ensure!(l.starts_with("https://"), "{link_key} must be an https:// link");
                }
                plans.push(StripePlan { tier, interval, price_id, legacy_price_ids, link });
            }
        }
        anyhow::ensure!(
            !plans.is_empty(),
            "STRIPE_WEBHOOK_SECRET is set but no STRIPE_PRICE_* variable is: the webhook could not tell which plan was bought"
        );

        let portal_url = nonblank("STRIPE_PORTAL_URL");
        if let Some(u) = &portal_url {
            anyhow::ensure!(u.starts_with("https://"), "STRIPE_PORTAL_URL must be an https:// link");
        }
        Ok(Some(Self { webhook_secret, plans, portal_url }))
    }
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

impl Environment {
    /// The deployment environment from the `ENVIRONMENT` variable.
    ///
    /// Unset means development. Any other value must be one of `development`, `staging` or
    /// `production` (case and surrounding whitespace ignored). An unrecognised value is an
    /// error, never a silent fallback: `ENVIRONMENT=prod` used to become Development, which
    /// quietly dropped the Secure flag on cookies and HSTS in a real deployment.
    pub fn parse(raw: Option<&str>) -> Result<Self> {
        match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            None => Ok(Environment::Development),
            Some("development") => Ok(Environment::Development),
            Some("staging") => Ok(Environment::Staging),
            Some("production") => Ok(Environment::Production),
            Some(_) => anyhow::bail!(
                "ENVIRONMENT must be one of: development, staging, production"
            ),
        }
    }
}

/// Whether an allowed-origin entry points at this machine.
fn is_local_origin(origin: &str) -> bool {
    let rest = origin.split_once("://").map(|(_, r)| r).unwrap_or(origin);
    let host = rest.split(['/', ':']).next().unwrap_or("");
    matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "[::1]")
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

        let environment = Environment::parse(get("ENVIRONMENT").as_deref())?;

        let configured_origins: Vec<String> = get("ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        // Outside development the browser origins must be stated, and in production they
        // must be real HTTPS origins. The localhost default exists for local work only; it
        // must not be what a deployed service silently runs with.
        if environment != Environment::Development {
            anyhow::ensure!(
                !configured_origins.is_empty(),
                "ALLOWED_ORIGINS must be set (comma-separated origins) when ENVIRONMENT is not development"
            );
        }
        if environment == Environment::Production {
            for origin in &configured_origins {
                anyhow::ensure!(
                    origin.starts_with("https://") && !is_local_origin(origin),
                    "ALLOWED_ORIGINS entries must be https:// origins that are not localhost in production"
                );
            }
        }
        let allowed_origins = if configured_origins.is_empty() {
            vec!["http://localhost:3000".to_string()]
        } else {
            configured_origins
        };

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
            environment,
            allowed_origins,
            otlp_endpoint: get("OTLP_ENDPOINT"),
            trust_proxy_headers: env_parse(&get, "TRUST_PROXY_HEADERS", false)?,
            trusted_proxy_hops: {
                let hops: usize = env_parse(&get, "TRUSTED_PROXY_HOPS", 1)?;
                anyhow::ensure!((1..=10).contains(&hops), "TRUSTED_PROXY_HOPS must be between 1 and 10");
                hops
            },
            rate_limit_general_per_second: env_parse(&get, "RATE_LIMIT_GENERAL_PER_SECOND", 20)?,
            rate_limit_general_burst: env_parse(&get, "RATE_LIMIT_GENERAL_BURST", 60)?,
            rate_limit_auth_per_minute: env_parse(&get, "RATE_LIMIT_AUTH_PER_MINUTE", 5)?,
            rate_limit_auth_burst: env_parse(&get, "RATE_LIMIT_AUTH_BURST", 5)?,
            scan_recovery_interval_secs: env_parse(&get, "SCAN_RECOVERY_INTERVAL_SECS", 60)?,
            scan_stale_after_secs: env_parse(&get, "SCAN_STALE_AFTER_SECS", 600)?,
            scan_redispatch_after_secs: env_parse(&get, "SCAN_REDISPATCH_AFTER_SECS", 600)?,
            scan_max_attempts: env_parse(&get, "SCAN_MAX_ATTEMPTS", 3)?,
            stripe: StripeConfig::from_lookup(&get)?,
            deletion_requests_enabled: env_parse(&get, "DELETION_REQUESTS_ENABLED", false)?,
            max_accounts: match get("MAX_ACCOUNTS").filter(|v| !v.trim().is_empty()) {
                Some(v) => Some(v.trim().parse::<i64>().ok().filter(|n| *n >= 0).context("MAX_ACCOUNTS must be a whole number, 0 or more")?),
                None => None,
            },
        })
    }

    pub fn is_production(&self) -> bool {
        self.environment == Environment::Production
    }

    /// Cookies get the `Secure` flag everywhere except local development: staging and
    /// production are served over HTTPS.
    pub fn secure_cookies(&self) -> bool {
        self.environment != Environment::Development
    }

    /// Send `Strict-Transport-Security`. Production only: HSTS with `preload` on a
    /// staging hostname would be hard to take back.
    pub fn hsts_enabled(&self) -> bool {
        self.is_production()
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
        assert_eq!(c.trusted_proxy_hops, 1);
        assert_eq!((c.rate_limit_auth_per_minute, c.rate_limit_auth_burst), (5, 5));
        assert_eq!(c.scan_max_attempts, 3);
        assert!(!c.is_production());
    }

    #[test]
    fn the_account_cap_is_optional_and_must_be_a_number() {
        assert_eq!(load(&good()).unwrap().max_accounts, None, "no cap by default");
        let mut v = good();
        v.insert("MAX_ACCOUNTS", "".into());
        assert_eq!(load(&v).unwrap().max_accounts, None, "blank means no cap");
        v.insert("MAX_ACCOUNTS", " 25 ".into());
        assert_eq!(load(&v).unwrap().max_accounts, Some(25));
        for bad in ["many", "-1", "2.5"] {
            v.insert("MAX_ACCOUNTS", bad.into());
            assert!(load(&v).is_err(), "{bad}");
        }
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
        v.insert("ALLOWED_ORIGINS", "https://app.example.com".into());
        let c = load(&v).unwrap();
        assert!(c.trust_proxy_headers && c.is_production());
        assert_eq!(c.rate_limit_auth_per_minute, 12);

        v.insert("SCAN_MAX_ATTEMPTS", "three".into());
        let err = load(&v).unwrap_err().to_string();
        assert!(err.contains("SCAN_MAX_ATTEMPTS"), "{err}");
    }

    // ---- ENVIRONMENT ---------------------------------------------------------------

    fn env_is(value: &str) -> Result<AppConfig> {
        let mut v = good();
        v.insert("ENVIRONMENT", value.to_string());
        v.insert("ALLOWED_ORIGINS", "https://app.example.com".into());
        load(&v)
    }

    #[test]
    fn unset_means_development() {
        assert_eq!(load(&good()).unwrap().environment, Environment::Development);
    }

    #[test]
    fn the_three_environments_parse_regardless_of_case_and_spacing() {
        assert_eq!(env_is("production").unwrap().environment, Environment::Production);
        assert_eq!(env_is("Production").unwrap().environment, Environment::Production);
        assert_eq!(env_is("  STAGING ").unwrap().environment, Environment::Staging);
        assert_eq!(env_is("development").unwrap().environment, Environment::Development);
    }

    #[test]
    fn an_unrecognised_environment_is_an_error_not_development() {
        // "prod" used to become Development: no Secure cookies, no HSTS, in production.
        for bad in ["prod", "prd", "live", "production1", "", "true"] {
            let err = env_is(bad).unwrap_err().to_string();
            assert!(err.contains("ENVIRONMENT"), "{bad}: {err}");
        }
    }

    #[test]
    fn cookies_are_secure_and_hsts_is_on_outside_development() {
        let dev = load(&good()).unwrap();
        assert!(!dev.secure_cookies() && !dev.hsts_enabled());
        let staging = env_is("staging").unwrap();
        assert!(staging.secure_cookies() && !staging.hsts_enabled(), "no preload-style HSTS on staging");
        let prod = env_is("production").unwrap();
        assert!(prod.secure_cookies() && prod.hsts_enabled());
    }

    #[test]
    fn outside_development_the_browser_origins_must_be_stated() {
        for env in ["staging", "production"] {
            let mut v = good();
            v.insert("ENVIRONMENT", env.into());
            let err = load(&v).unwrap_err().to_string();
            assert!(err.contains("ALLOWED_ORIGINS"), "{env}: {err}");
            v.insert("ALLOWED_ORIGINS", " , ".into());
            assert!(load(&v).is_err(), "{env}: blank entries are not origins");
        }
    }

    #[test]
    fn production_origins_must_be_https_and_not_local() {
        for bad in [
            "http://app.example.com",
            "https://localhost:3000",
            "https://127.0.0.1",
            "https://app.example.com,http://localhost:3000",
        ] {
            let mut v = good();
            v.insert("ENVIRONMENT", "production".into());
            v.insert("ALLOWED_ORIGINS", bad.into());
            let err = load(&v).unwrap_err().to_string();
            assert!(err.contains("ALLOWED_ORIGINS"), "{bad}: {err}");
        }
        let mut v = good();
        v.insert("ENVIRONMENT", "production".into());
        v.insert("ALLOWED_ORIGINS", "https://app.example.com, https://www.example.com".into());
        assert_eq!(load(&v).unwrap().allowed_origins.len(), 2);
    }

    #[test]
    fn trusted_proxy_hops_must_be_a_small_positive_number() {
        let mut v = good();
        v.insert("TRUSTED_PROXY_HOPS", "2".into());
        assert_eq!(load(&v).unwrap().trusted_proxy_hops, 2);
        for bad in ["0", "11", "-1", "many", ""] {
            v.insert("TRUSTED_PROXY_HOPS", bad.into());
            let err = load(&v).unwrap_err().to_string();
            assert!(err.contains("TRUSTED_PROXY_HOPS"), "{bad}: {err}");
        }
    }

    #[test]
    fn development_keeps_its_localhost_default() {
        assert_eq!(load(&good()).unwrap().allowed_origins, vec!["http://localhost:3000".to_string()]);
    }

    // ---- Stripe ---------------------------------------------------------------------

    fn with_stripe() -> HashMap<&'static str, String> {
        let mut v = good();
        v.insert("STRIPE_WEBHOOK_SECRET", "whsec_0123456789abcdef".into());
        v.insert("STRIPE_PRICE_PERSONAL_MONTHLY", "price_pm".into());
        v.insert("STRIPE_PRICE_PERSONAL_ANNUAL", "price_py".into());
        v.insert("STRIPE_PRICE_PREMIUM_MONTHLY", "price_rm".into());
        v.insert("STRIPE_PRICE_PREMIUM_ANNUAL", "price_ry".into());
        v.insert("STRIPE_LINK_PERSONAL_MONTHLY", "https://buy.stripe.com/pm".into());
        v
    }

    #[test]
    fn billing_is_off_unless_a_webhook_secret_is_set() {
        assert!(load(&good()).unwrap().stripe.is_none());
        let mut v = good();
        v.insert("STRIPE_WEBHOOK_SECRET", "   ".into());
        assert!(load(&v).unwrap().stripe.is_none(), "blank counts as unset");
    }

    #[test]
    fn all_four_plans_are_read_with_their_tier_and_interval() {
        let s = load(&with_stripe()).unwrap().stripe.unwrap();
        assert_eq!(s.plans.len(), 4);
        let find = |id: &str| s.plans.iter().find(|p| p.sells(id)).unwrap();
        assert_eq!((find("price_pm").tier, find("price_pm").interval), (PaidTier::Personal, BillingInterval::Month));
        assert_eq!((find("price_py").tier, find("price_py").interval), (PaidTier::Personal, BillingInterval::Year));
        assert_eq!((find("price_rm").tier, find("price_rm").interval), (PaidTier::Premium, BillingInterval::Month));
        assert_eq!((find("price_ry").tier, find("price_ry").interval), (PaidTier::Premium, BillingInterval::Year));
        assert!(find("price_pm").link.is_some() && find("price_py").link.is_none());
        assert!(PaidTier::Premium > PaidTier::Personal);
    }

    #[test]
    fn bad_stripe_settings_fail_at_startup_naming_the_variable_not_the_value() {
        let cases: [(&str, &str); 6] = [
            ("STRIPE_WEBHOOK_SECRET", "zq9-tiny"),
            ("STRIPE_PRICE_PREMIUM_MONTHLY", "prod_wrong_kind_of_id"),
            ("STRIPE_PRICE_PREMIUM_ANNUAL", "price_pm"), // repeats another plan's price
            ("STRIPE_LINK_PERSONAL_MONTHLY", "http://buy.stripe.com/insecure"),
            ("STRIPE_LINK_PREMIUM_MONTHLY", "ftp://x"),
            ("STRIPE_PORTAL_URL", "http://billing.stripe.com/p/login/x"),
        ];
        for (key, bad) in cases {
            let mut v = with_stripe();
            v.insert(key, bad.into());
            let err = load(&v).unwrap_err().to_string();
            assert!(err.contains(key), "{key}: {err}");
            assert!(!err.contains(bad), "the value must not appear: {err}");
        }
    }

    #[test]
    fn older_prices_listed_after_the_current_one_are_still_honoured() {
        let mut v = with_stripe();
        v.insert("STRIPE_PRICE_PERSONAL_MONTHLY", " price_new , price_old1,price_old2 ".into());
        let s = load(&v).unwrap().stripe.unwrap();
        let plan = s.plans.iter().find(|p| p.price_id == "price_new").expect("the first id is the one sold");
        assert_eq!(plan.legacy_price_ids, vec!["price_old1".to_string(), "price_old2".to_string()]);
        assert!(plan.sells("price_new") && plan.sells("price_old1") && plan.sells("price_old2"));
        assert!(!plan.sells("price_rm"), "another plan's price is not this plan's");
        assert_eq!((plan.tier, plan.interval), (PaidTier::Personal, BillingInterval::Month));
    }

    #[test]
    fn a_price_cannot_belong_to_two_plans_or_be_listed_twice() {
        for (key, bad) in [
            ("STRIPE_PRICE_PERSONAL_MONTHLY", "price_pm,price_rm"),        // another plan's current price
            ("STRIPE_PRICE_PERSONAL_MONTHLY", "price_pm,price_ry"),        // another plan's price
            ("STRIPE_PRICE_PERSONAL_MONTHLY", "price_pm,price_pm"),        // twice in one list
            ("STRIPE_PRICE_PERSONAL_MONTHLY", "price_pm,price_x,price_x"),
            ("STRIPE_PRICE_PERSONAL_MONTHLY", "price_pm,not_a_price"),     // every id must be a price id
        ] {
            let mut v = with_stripe();
            v.insert(key, bad.into());
            let err = load(&v).unwrap_err().to_string();
            // names a STRIPE_PRICE_* variable (the one parsed second, when the clash is found)
            assert!(err.contains("STRIPE_PRICE_") && !err.contains(bad), "{bad}: {err}");
            assert!(err.contains("repeats") || err.contains("twice") || err.contains("must list"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_webhook_secret_without_any_price_is_refused() {
        let mut v = good();
        v.insert("STRIPE_WEBHOOK_SECRET", "whsec_0123456789abcdef".into());
        let err = load(&v).unwrap_err().to_string();
        assert!(err.contains("STRIPE_PRICE"), "{err}");
    }
}

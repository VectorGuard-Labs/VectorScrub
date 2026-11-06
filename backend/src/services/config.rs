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
    pub port: u16,
    pub db_pool_size: u32,
    pub agent_service_url: String,
    pub environment: Environment,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Environment {
    Development,
    Staging,
    Production,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_url: required("DATABASE_URL")?,
            redis_url: env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into()),
            jwt_secret: required("JWT_SECRET")?,
            master_key_b64: required("MASTER_ENCRYPTION_KEY")?,
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".into())
                .parse()
                .context("PORT must be a valid port number")?,
            db_pool_size: env::var("DB_POOL_SIZE")
                .unwrap_or_else(|_| "10".into())
                .parse()
                .context("DB_POOL_SIZE must be a number")?,
            agent_service_url: env::var("AGENT_SERVICE_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8000".into()),
            environment: match env::var("ENVIRONMENT").as_deref() {
                Ok("production") => Environment::Production,
                Ok("staging") => Environment::Staging,
                _ => Environment::Development,
            },
        })
    }

    pub fn is_production(&self) -> bool {
        self.environment == Environment::Production
    }
}

fn required(key: &str) -> Result<String> {
    env::var(key).with_context(|| format!("Required environment variable '{key}' is not set"))
}

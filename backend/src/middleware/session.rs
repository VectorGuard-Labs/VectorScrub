//! The browser session cookie.
//!
//! The signed JWT used to be returned in the login response body and kept by the frontend in
//! `localStorage`, where any script that runs on the page (one XSS bug, one compromised
//! dependency) can read it and use it for 24 hours. It now travels only in an `HttpOnly`
//! cookie that scripts cannot read:
//!
//!   * `HttpOnly`           JavaScript cannot read it, so XSS cannot steal it.
//!   * `Secure`             HTTPS only (everywhere except local development).
//!   * `SameSite=Strict`    never sent on cross-site requests.
//!   * `Path=/api/v1`       only sent to the API, not to every page request.
//!   * `Max-Age` = the token's lifetime, so the cookie and the JWT expire together.
//!
//! State-changing requests made with this cookie are still protected by the double-submit
//! CSRF check (`middleware::csrf`), which applies to every non-GET request regardless of how
//! it authenticates. The `Authorization: Bearer` header remains accepted for non-browser
//! clients and takes precedence when both are present.
//!
//! Limits worth knowing: XSS can still make requests *as* the user while the page is open
//! (HttpOnly stops theft of the token, not use of the session), and there is no server-side
//! revocation, so logging out clears the cookie but a token copied before that stays valid
//! until it expires.

use axum_extra::extract::cookie::{Cookie, SameSite};
use time::Duration;

pub const SESSION_COOKIE: &str = "vs_session";

/// Must match the lifetime `services::auth::issue_token` gives the JWT.
pub const SESSION_MAX_AGE_HOURS: i64 = 24;

const COOKIE_PATH: &str = "/api/v1";

/// The cookie that carries a freshly issued token.
pub fn session_cookie(token: &str, secure: bool) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token.to_string()))
        .path(COOKIE_PATH)
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Strict)
        .max_age(Duration::hours(SESSION_MAX_AGE_HOURS))
        .build()
}

/// A cookie that tells the browser to drop the session. Same name, path and flags, or the
/// browser would treat it as a different cookie and keep the real one.
pub fn cleared_session_cookie(secure: bool) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, ""))
        .path(COOKIE_PATH)
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Strict)
        .max_age(Duration::ZERO)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_cookie_cannot_be_read_by_scripts_or_sent_cross_site() {
        let c = session_cookie("tok", true).to_string();
        assert!(c.contains("HttpOnly"), "{c}");
        assert!(c.contains("Secure"), "{c}");
        assert!(c.contains("SameSite=Strict"), "{c}");
        assert!(c.contains("Path=/api/v1"), "{c}");
        assert!(c.contains("Max-Age=86400"), "{c}");
        assert!(c.starts_with("vs_session=tok"), "{c}");
    }

    #[test]
    fn development_cookies_are_not_secure_but_are_still_http_only() {
        let c = session_cookie("tok", false).to_string();
        assert!(!c.contains("Secure"), "{c}");
        assert!(c.contains("HttpOnly") && c.contains("SameSite=Strict"), "{c}");
    }

    #[test]
    fn logout_expires_the_same_cookie() {
        let c = cleared_session_cookie(true).to_string();
        assert!(c.starts_with("vs_session=;"), "{c}");
        assert!(c.contains("Max-Age=0") && c.contains("Path=/api/v1") && c.contains("HttpOnly"), "{c}");
    }
}

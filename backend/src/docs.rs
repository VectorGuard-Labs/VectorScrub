use utoipa::OpenApi;

use crate::api::auth::{AuthResponse, LoginRequest};
use crate::services::billing::{BillingView, PlanView, SubscriptionView};
use crate::models::customer::{CreateAccountRequest, CustomerProfile, SubscriptionTier};

/// OpenAPI documentation for VectorScrub API
#[derive(OpenApi)]
#[openapi(
    info(
        title = "VectorScrub API",
        version = "0.1.0",
        description = "Privacy protection API for discovering and removing personal information from the web",
        contact(
            name = "VectorGuard Labs",
            email = "support@vectorguardlabs.com"
        ),
        license(
            name = "Proprietary",
        )
    ),
    paths(
        crate::api::auth::register,
        crate::api::auth::login,
        crate::api::auth::logout,
        crate::api::billing::get_billing,
        crate::api::billing::stripe_webhook,
    ),
    components(
        schemas(
            CreateAccountRequest,
            LoginRequest,
            AuthResponse,
            CustomerProfile,
            SubscriptionTier,
            BillingView,
            PlanView,
            SubscriptionView,
        )
    ),
    tags(
        (name = "auth", description = "Authentication endpoints"),
        (name = "customers", description = "Customer profile management"),
        (name = "billing", description = "Plans, subscription state and the Stripe webhook"),
        (name = "scans", description = "Privacy scan operations"),
        (name = "exposures", description = "Data exposure management"),
        (name = "removals", description = "Removal request operations"),
    ),
    servers(
        (url = "http://localhost:8080", description = "Local development"),
        (url = "https://api.vectorscrub.com", description = "Production"),
    )
)]
pub struct ApiDoc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_billing_endpoints_are_documented() {
        let doc = serde_json::to_value(ApiDoc::openapi()).unwrap();
        let paths = doc["paths"].as_object().unwrap();
        assert!(paths.contains_key("/api/v1/me/billing"), "{:?}", paths.keys().collect::<Vec<_>>());
        assert!(paths.contains_key("/api/v1/webhooks/stripe"));
        assert!(doc["components"]["schemas"].get("BillingView").is_some());
    }
}

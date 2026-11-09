use utoipa::OpenApi;

use crate::api::auth::{AuthResponse, LoginRequest};
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
    ),
    components(
        schemas(
            CreateAccountRequest,
            LoginRequest,
            AuthResponse,
            CustomerProfile,
            SubscriptionTier,
        )
    ),
    tags(
        (name = "auth", description = "Authentication endpoints"),
        (name = "customers", description = "Customer profile management"),
        (name = "scans", description = "Privacy scan operations"),
        (name = "exposures", description = "Data exposure management"),
        (name = "removals", description = "Removal request operations"),
        (name = "family", description = "Family plan management"),
    ),
    servers(
        (url = "http://localhost:8080", description = "Local development"),
        (url = "https://api.vectorscrub.com", description = "Production"),
    )
)]
pub struct ApiDoc;

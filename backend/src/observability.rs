use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use opentelemetry::trace::TracerProvider;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{runtime, trace as sdktrace, Resource};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Initialize OpenTelemetry tracing and Prometheus metrics
///
/// This sets up:
/// - Distributed tracing with OpenTelemetry (optional OTLP export)
/// - Prometheus metrics exporter
/// - Structured logging with tracing
pub fn init_observability(service_name: &str, otlp_endpoint: Option<&str>) -> PrometheusHandle {
    // Initialize Prometheus metrics exporter
    let prometheus_handle = PrometheusBuilder::new()
        .install_recorder()
        .expect("Failed to install Prometheus recorder");

    // Create OpenTelemetry resource with service information
    let resource = Resource::new(vec![
        KeyValue::new("service.name", service_name.to_string()),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
    ]);

    // Set up tracing layers
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,vectorscrub_backend=debug"));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .json();

    // Optionally add OpenTelemetry tracing if OTLP endpoint is provided
    if let Some(endpoint) = otlp_endpoint {
        match setup_otlp_tracing(endpoint, resource) {
            Ok(tracer) => {
                let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt_layer)
                    .with(telemetry_layer)
                    .init();

                tracing::info!("OpenTelemetry tracing initialized with OTLP endpoint: {}", endpoint);
            }
            Err(e) => {
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt_layer)
                    .init();

                tracing::warn!("Failed to initialize OTLP tracing: {}. Continuing without it.", e);
            }
        }
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt_layer)
            .init();

        tracing::info!("Observability initialized (tracing + Prometheus metrics)");
    }

    // Record custom metrics
    register_custom_metrics();

    prometheus_handle
}

/// Set up OpenTelemetry tracing with OTLP exporter
fn setup_otlp_tracing(
    endpoint: &str,
    resource: Resource,
) -> Result<sdktrace::Tracer, Box<dyn std::error::Error>> {
    use opentelemetry_otlp::SpanExporter;

    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;

    let tracer_provider = sdktrace::TracerProvider::builder()
        .with_batch_exporter(exporter, runtime::Tokio)
        .with_resource(resource)
        .build();

    let tracer = tracer_provider.tracer("vectorscrub-backend");

    Ok(tracer)
}

/// Register custom application metrics
fn register_custom_metrics() {
    use metrics::{describe_counter, describe_gauge, describe_histogram};

    // Request metrics
    describe_counter!(
        "http_requests_total",
        "Total number of HTTP requests received"
    );
    describe_histogram!(
        "http_request_duration_seconds",
        "HTTP request latency in seconds"
    );
    describe_counter!(
        "http_requests_errors_total",
        "Total number of HTTP request errors"
    );

    // Database metrics
    describe_gauge!(
        "db_connections_active",
        "Number of active database connections"
    );
    describe_histogram!(
        "db_query_duration_seconds",
        "Database query latency in seconds"
    );

    // Business metrics
    describe_counter!(
        "scans_initiated_total",
        "Total number of scans initiated"
    );
    describe_counter!(
        "exposures_discovered_total",
        "Total number of exposures discovered"
    );
    describe_counter!(
        "removal_requests_total",
        "Total number of removal requests"
    );

    // Job queue metrics
    describe_gauge!("redis_queue_length", "Number of jobs in Redis queue");
    describe_counter!(
        "jobs_processed_total",
        "Total number of jobs processed"
    );
    describe_counter!("jobs_failed_total", "Total number of failed jobs");

    // Agent metrics
    describe_counter!(
        "agent_api_calls_total",
        "Total number of agent API calls"
    );
    describe_histogram!(
        "agent_callback_duration_seconds",
        "Agent callback duration in seconds"
    );
}

/// Shutdown OpenTelemetry gracefully
pub fn shutdown_observability() {
    opentelemetry::global::shutdown_tracer_provider();
}

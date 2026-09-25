//! Observability: tracing setup, OTLP export, and DecisionRecord logging.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

// Re-export for convenience so dependents don't need vkdg-core directly.
pub use vkdg_core::DecisionRecord;

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Pretty,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserveConfig {
    pub otlp_endpoint: Option<String>,
    #[serde(default)]
    pub log_format: LogFormat,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Default for ObserveConfig {
    fn default() -> Self {
        Self {
            otlp_endpoint: None,
            log_format: LogFormat::Pretty,
            log_level: default_log_level(),
        }
    }
}

// ── Tracing init ──────────────────────────────────────────────────────────────

/// Initialise the global tracing subscriber.
///
/// Always installs an EnvFilter-gated stderr layer.
/// When `config.otlp_endpoint` is `Some`, also attaches an OpenTelemetry layer
/// (added first on the Registry so the type parameter `S = Registry` is satisfied).
/// OTLP errors are demoted to a warning — this function never panics.
pub fn init_tracing(config: &ObserveConfig) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.log_level));

    // Try to build the OTel layer; failure is non-fatal.
    let otel = config
        .otlp_endpoint
        .as_deref()
        .and_then(|ep| build_otel_layer(ep).ok());

    let otel_skipped = config.otlp_endpoint.is_some() && otel.is_none();

    // OTel layer must be added first (directly on Registry) so that its
    // `S = Registry` type parameter is satisfied.
    match (config.log_format, otel) {
        (LogFormat::Json, Some(otel_layer)) => tracing_subscriber::registry()
            .with(otel_layer)
            .with(filter)
            .with(fmt::layer().json())
            .try_init()
            .context("failed to set global tracing subscriber")?,
        (LogFormat::Json, None) => tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().json())
            .try_init()
            .context("failed to set global tracing subscriber")?,
        (LogFormat::Pretty, Some(otel_layer)) => tracing_subscriber::registry()
            .with(otel_layer)
            .with(filter)
            .with(fmt::layer().pretty())
            .try_init()
            .context("failed to set global tracing subscriber")?,
        (LogFormat::Pretty, None) => tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().pretty())
            .try_init()
            .context("failed to set global tracing subscriber")?,
    }

    if otel_skipped {
        tracing::warn!("OTLP layer disabled: exporter setup failed");
    }

    Ok(())
}

fn build_otel_layer(
    endpoint: &str,
) -> anyhow::Result<
    tracing_opentelemetry::OpenTelemetryLayer<
        tracing_subscriber::Registry,
        opentelemetry_sdk::trace::Tracer,
    >,
> {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::trace::TracerProvider;

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .context("building OTLP span exporter")?;

    let provider = TracerProvider::builder()
        .with_simple_exporter(exporter)
        .build();

    opentelemetry::global::set_tracer_provider(provider.clone());
    let tracer = provider.tracer("vkdg");
    Ok(tracing_opentelemetry::layer().with_tracer(tracer))
}

// ── DecisionRecord exporter ───────────────────────────────────────────────────

/// Fire-and-forget decision record sink.
///
/// Serialises the record to JSON and emits it as a structured tracing event.
/// Phase B will replace this with an async channel + background export.
pub struct DecisionRecordExporter;

impl DecisionRecordExporter {
    pub fn new() -> Self {
        Self
    }

    /// Emit `record` as a structured tracing event. Never panics.
    pub fn export(&self, record: &DecisionRecord) {
        match serde_json::to_string(record) {
            Ok(json) => tracing::info!(decision_record = %json, "decision"),
            Err(e) => tracing::error!(error = %e, "failed to serialise DecisionRecord"),
        }
    }
}

impl Default for DecisionRecordExporter {
    fn default() -> Self {
        Self::new()
    }
}

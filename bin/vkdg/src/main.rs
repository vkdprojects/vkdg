//! VKDG gateway binary entry point.

use std::sync::Arc;

use anyhow::Result;
use axum::routing::{get, post};
use axum::Router;
use clap::{Parser, Subcommand};
use vkdg_connections::{
    AuthKind, ConnectionCatalog, ConnectionConfig, CredentialManager, ProviderKind,
};
use vkdg_core::ConnectionId;
use vkdg_http::{AdmissionGuard, AppState, PipelineState, ServerConfig};
use vkdg_http::upstream::HttpClient;
use vkdg_observe::{init_tracing, DecisionRecordExporter, ObserveConfig};
use vkdg_operations::CapabilitySet;
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router as VkdgRouter, StrategyKind};
use vkdg_provider_anthropic::AnthropicAdapter;
use vkdg_config::{ConfigSnapshot, load_and_validate};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "vkdg", version = "0.1.0", about = "VKDG AI gateway")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long, short)]
        config: Option<String>,
        #[arg(long, default_value = "0.0.0.0:8080")]
        listen: String,
    },
    Doctor,
    Config {
        #[command(subcommand)]
        sub: ConfigSub,
    },
    /// Inspect requests processed by the gateway.
    Request {
        #[command(subcommand)]
        sub: RequestSub,
    },
    /// Replay a recorded request fixture against the gateway.
    Replay {
        /// Path to the YAML fixture file.
        fixture: String,
        /// Override the gateway base URL (default: VKDG_BASE_URL or http://127.0.0.1:8080).
        #[arg(long)]
        base_url: Option<String>,
    },
}

#[derive(Subcommand)]
enum RequestSub {
    /// Show routing decision and phases for a request.
    Explain {
        /// Request ID (UUID)
        id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ConfigSub {
    Check { path: String },
    /// Simulate routing for a model using live eligibility logic.
    Explain {
        /// Model name or combo name to simulate routing for
        #[arg(long)]
        model: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve { config, listen } => serve(config, listen).await?,
        Command::Doctor => vkdg_cli::commands::doctor::run().await?,
        Command::Config { sub: ConfigSub::Check { path } } => {
            vkdg_cli::commands::config_check::run(&path).await?;
        }
        Command::Config { sub: ConfigSub::Explain { model, json } } => {
            vkdg_cli::commands::config_explain::run(&model, json).await?;
        }
        Command::Request { sub: RequestSub::Explain { id, json } } => {
            vkdg_cli::commands::explain::run(&id, json).await?;
        }
        Command::Replay { fixture, base_url } => {
            vkdg_cli::commands::replay::run(&fixture, base_url.as_deref()).await?;
        }
    }
    Ok(())
}

// ── Serve ─────────────────────────────────────────────────────────────────────

async fn serve(config_path: Option<String>, listen: String) -> Result<()> {
    let _ = init_tracing(&ObserveConfig { otlp_endpoint: None, ..Default::default() });

    let server_config = ServerConfig { listen_addr: listen.clone(), ..Default::default() };

    let max_concurrent: usize = std::env::var("VKDG_MAX_CONCURRENT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);

    let pipeline = if let Some(path) = &config_path {
        match load_and_validate(path, 1) {
            Ok(snap) => {
                tracing::info!(path = %path, version = snap.version, "loaded config from file");
                let pipeline = build_pipeline_from_snapshot(&snap, max_concurrent);
                // Install hot-reload watcher; errors on bad reloads are logged, not fatal.
                let (tx, _rx) = vkdg_config::config_channel(snap);
                let _ = vkdg_config::watch(path.clone(), tx, 1);
                Some(pipeline)
            }
            Err(e) => {
                tracing::warn!(error = %e, "config file invalid — falling back to env vars");
                build_pipeline_from_env(max_concurrent)
            }
        }
    } else {
        build_pipeline_from_env(max_concurrent)
    };

    let mut state = AppState::new(server_config.clone());
    if let Some(p) = pipeline {
        state = state.with_pipeline(Arc::new(p));
    }

    // Build router here (not via vkdg_http::build_router) so we can add all
    // routes before calling .with_state() once. This avoids the Router<S> type
    // mismatch that comes from calling .route() on an already-resolved Router<()>.
    let router: Router = Router::new()
        .route("/v1/messages", post(vkdg_ingress_anthropic::handle_messages))
        .route("/v1/chat/completions", post(vkdg_ingress_openai::handle_chat_completions))
        .route("/v1/images/generations", post(vkdg_ingress_openai::handle_image_generations))
        .route("/health", get(health))
        .route("/vkdg/v1/info", get(info))
        .route("/mcp", get(vkdg_http::mcp_discovery))
        .with_state(state);

    // ── Admin API on a separate port ───────────────────────────────────────────
    let admin_addr = std::env::var("VKDG_ADMIN_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9090".into());
    let bootstrap_token = std::env::var("VKDG_BOOTSTRAP_TOKEN").unwrap_or_else(|_| {
        let t = uuid::Uuid::new_v4().to_string();
        println!("Bootstrap token (one-time): {t}");
        t
    });
    let dummy_snap = vkdg_config::ConfigSnapshot::default_empty();
    let (_config_tx, config_rx) = vkdg_config::config_channel(dummy_snap);
    let admin_state = vkdg_admin::AdminState {
        sessions: vkdg_admin::session::SessionStore::new(bootstrap_token),
        config_rx,
        started_at: std::sync::Arc::new(std::time::Instant::now()),
        key_store: vkdg_admin::session::KeyStore::new(),
        request_log: vkdg_admin::handlers::requests::RequestLog::new(),
        combo_resolver: None,
        catalog: None,
    };
    let admin_router = vkdg_admin::build_admin_router(admin_state);
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(&admin_addr)
            .await
            .expect("bind admin listener");
        tracing::info!(addr = %admin_addr, "admin API listening");
        axum::serve(listener, admin_router).await.ok();
    });

    println!("VKDG gateway listening on {listen}");
    tracing::info!(addr = %listen, "vkdg starting");

    vkdg_http::serve(server_config, router).await
}
async fn health() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({"status": "ok"}))
}

async fn info() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({"version": "0.1.0"}))
}

// ── Pipeline builder — from ConfigSnapshot ────────────────────────────────────

fn build_pipeline_from_snapshot(snap: &ConfigSnapshot, max_concurrent: usize) -> PipelineState {
    let admission = Arc::new(AdmissionGuard::new(
        snap.limits.max_concurrent_requests.unwrap_or(max_concurrent),
    ));
    let router = Arc::new(VkdgRouter::new(snap.routes.as_ref().clone()));
    let catalog = Arc::new(ConnectionCatalog::new(snap.connections.as_ref().clone()));
    PipelineState {
        admission,
        router,
        catalog,
        credentials: Arc::new(CredentialManager::new()),
        http_client: Arc::new(HttpClient::new()),
        exporter: Arc::new(DecisionRecordExporter::new()),
        provider_adapter: Arc::new(AnthropicAdapter),
        cache: None,
        combo_resolver: None,
        compressor: None,
        dedup_table: None,
        session_registry: None,
        quota_tracker: None,
        global_system_prompt: snap.gateway.global_system_prompt.clone(),
        ip_policy: None,
        latency_tracker: None,
        memory_store: None,
        eval_enabled: false,
    }
}

// ── Pipeline builder — from env vars (fallback) ───────────────────────────────

fn build_pipeline_from_env(max_concurrent: usize) -> Option<PipelineState> {
    let api_key_var = "ANTHROPIC_API_KEY";
    if std::env::var(api_key_var).is_err() {
        eprintln!("ANTHROPIC_API_KEY not set — pipeline disabled, /v1/messages returns 501");
        return None;
    }

    let conn_id = ConnectionId("anthropic-default".into());

    let config = ConnectionConfig {
        id: conn_id.clone(),
        provider: ProviderKind::Anthropic,
        auth: AuthKind::ApiKey { env_var: api_key_var.into() },
        models: vec!["claude-*".into()],
        max_concurrent: max_concurrent as u32,
        weight: 1,
        tags: vec![],
        capabilities: CapabilitySet::default(),
    };

    let route = RouteConfig {
        id: RouteId("default".into()),
        match_models: vec!["claude-*".into()],
        strategy: StrategyKind::RoundRobin,
        targets: vec![conn_id],
        plugin_hooks: PluginHooks::default(),
    };

    Some(PipelineState {
        admission: Arc::new(AdmissionGuard::new(max_concurrent)),
        router: Arc::new(VkdgRouter::new(vec![route])),
        catalog: Arc::new(ConnectionCatalog::new(vec![config])),
        credentials: Arc::new(CredentialManager::new()),
        http_client: Arc::new(HttpClient::new()),
        exporter: Arc::new(DecisionRecordExporter::new()),
        provider_adapter: Arc::new(AnthropicAdapter),
        cache: None,
        combo_resolver: None,
        compressor: None,
        dedup_table: None,
        session_registry: None,
        quota_tracker: None,
        global_system_prompt: None,
        ip_policy: None,
        latency_tracker: None,
        memory_store: None,
        eval_enabled: false,
    })
}

//! VKDG gateway binary entry point.

mod e2e_fake_oauth;

use std::sync::Arc;

use anyhow::Result;
use axum::routing::{get, post};
use axum::Router;
use clap::{Parser, Subcommand};
use vkdg_config::{load_and_validate, ConfigSnapshot};
use vkdg_connections::{
    Account, AccountStore, AuthKind, ConnectionCatalog, ConnectionConfig, CredentialManager,
    ProviderKind,
};
use vkdg_core::ConnectionId;
use vkdg_http::upstream::HttpClient;
use vkdg_http::{AdmissionGuard, AppState, PipelineState, ServerConfig};
use vkdg_observe::{init_tracing, DecisionRecordExporter, ObserveConfig};
use vkdg_operations::CapabilitySet;
use vkdg_provider_anthropic::AnthropicAdapter;
use vkdg_provider_antigravity::AntigravityAdapter;
use vkdg_provider_cerebras::provider as cerebras_provider;
use vkdg_provider_claude_code::ClaudeCodeAdapter;
use vkdg_provider_codex::CodexAdapter;
use vkdg_provider_deepseek::provider as deepseek_provider;
use vkdg_provider_fireworks::provider as fireworks_provider;
use vkdg_provider_gemini::provider as gemini_provider;
use vkdg_provider_github_copilot::GitHubCopilotAdapter;
use vkdg_provider_groq::provider as groq_provider;
use vkdg_provider_kimi_coding::KimiCodingAdapter;
use vkdg_provider_kiro::KiroAdapter;
use vkdg_provider_mistral::provider as mistral_provider;
use vkdg_provider_nvidia_nim::provider as nvidia_nim_provider;
use vkdg_provider_openai::OpenAIAdapter;
use vkdg_provider_sambanova::provider as sambanova_provider;
use vkdg_provider_sdk::{
    find_login_method, resolve_login_params, run_device_login, LoginParams, LoginResult, OAuthFlow,
    ProviderRegistry,
};
use vkdg_provider_together::provider as together_provider;
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router as VkdgRouter, StrategyKind};

// ── Clack visual language constants ──────────────────────────────────────────

const BAR: &str = "│";
const STEP_DONE: &str = "◇";
const STEP_ERR: &str = "▲";
// ── Embedded console assets ───────────────────────────────────────────────────

/// SvelteKit console — embedded at compile time in release builds,
/// served from the filesystem in debug builds for fast iteration.
#[derive(rust_embed::RustEmbed, Clone)]
#[folder = "../../apps/console/build/"]
struct ConsoleAssets;

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
        /// Bootstrap token (overrides VKDG_BOOTSTRAP_TOKEN env var).
        #[arg(long)]
        token: Option<String>,
        /// Suppress INFO logs (RUST_LOG=warn). Errors and warnings still show.
        #[arg(long)]
        quiet: bool,
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
        /// Start an in-process mock gateway instead of connecting to a running instance.
        #[arg(long)]
        self_test: bool,
    },
    /// First-run configuration wizard.
    Setup,
    /// Install VKDG as a systemd service (Linux only).
    Install {
        #[arg(long, default_value = "/etc/vkdg/config.yaml")]
        config: String,
        /// Install for current user only (no root required).
        #[arg(long)]
        user: bool,
    },
    /// Self-update to the latest release.
    Update {
        /// Skip confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Update to specific version.
        #[arg(long)]
        version: Option<String>,
    },
    /// Manage plugins from the VKDG registry.
    Plugin {
        #[command(subcommand)]
        sub: PluginSub,
    },
    /// Log in to an OAuth provider plugin and save the account.
    Login {
        /// Provider plugin id, e.g. `kiro`, `github-copilot`.
        provider: String,
        /// Login method id (default: the provider's first method).
        #[arg(long)]
        method: Option<String>,
        /// Plugin-defined option, repeatable: `--opt region=us-east-1`.
        #[arg(long = "opt", value_name = "KEY=VALUE")]
        opts: Vec<String>,
        /// PKCE only: authorization code or pasted callback URL (skips the prompt).
        #[arg(long)]
        code: Option<String>,
        /// Print the provider's login methods and exit.
        #[arg(long)]
        list_methods: bool,
    },
    /// Manage saved provider accounts.
    Accounts {
        #[command(subcommand)]
        sub: AccountsSub,
    },
    /// Manage client API keys for the data plane (`/v1/*`).
    Keys {
        #[command(subcommand)]
        sub: KeysSub,
    },
    /// Admin console sign-in.
    Admin {
        #[command(subcommand)]
        sub: AdminSub,
    },
}

#[derive(Subcommand)]
enum AdminSub {
    /// Set or replace the console password (reads it from stdin, or prompts).
    /// Use this to recover access; it needs shell access to the host.
    SetPassword,
}

#[derive(Subcommand)]
enum KeysSub {
    /// Create a key. The raw key is printed once and never stored.
    Create {
        name: String,
        /// Tenant the key belongs to.
        #[arg(long, default_value = "default")]
        tenant: String,
        /// Restrict to chat endpoints (no image generation).
        #[arg(long)]
        inference_only: bool,
        /// Only these model patterns, repeatable: `--model 'claude-*'`.
        #[arg(long = "model")]
        models: Vec<String>,
        /// Only from these addresses or CIDR ranges, repeatable: `--ip 10.0.0.0/8`.
        #[arg(long = "ip")]
        ips: Vec<String>,
        /// Stop working after this many days.
        #[arg(long)]
        expires_in_days: Option<u32>,
        /// Token budget (input + output) per calendar month, UTC.
        #[arg(long)]
        monthly_tokens: Option<u64>,
        /// Requests per minute.
        #[arg(long)]
        rpm: Option<u32>,
        /// Keep this key's requests out of the request history.
        #[arg(long)]
        no_log: bool,
    },
    /// List keys (prefix only; raw keys are never shown again).
    List,
    /// Revoke a key by id. Takes effect on running gateways within seconds.
    Revoke { id: String },
}

#[derive(Subcommand)]
enum AccountsSub {
    /// List saved accounts (tokens are never printed).
    List,
    /// Delete a saved account.
    Remove { id: String },
}

#[derive(Subcommand)]
enum PluginSub {
    /// Search the registry for plugins.
    Search {
        query: Option<String>,
        #[arg(long, value_enum)]
        kind: Option<PluginKind>,
    },
    /// Install a plugin from the registry or a URL.
    Install {
        /// Registry name (`name` or `name@version`), a local plugin directory
        /// (`manifest.yaml` + `plugin.wasm`), a manifest file, or a manifest URL.
        plugin: String,
        #[arg(long, default_value = "main")]
        registry_ref: String,
    },
    /// List installed plugins.
    List,
    /// Remove an installed plugin.
    Remove { name: String },
    /// Update all installed plugins to latest versions.
    Update,
    /// Add a third-party plugin registry.
    Tap {
        /// GitHub repo: owner/repo, or --list to show current taps.
        #[arg(conflicts_with = "list")]
        repo: Option<String>,
        #[arg(long)]
        list: bool,
    },
    /// Validate a plugin manifest file (used by CI).
    ValidateManifest { path: String },
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum PluginKind {
    Provider,
    OauthProvider,
    FilterPack,
    Compressor,
    Router,
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
    Check {
        path: String,
    },
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
        Command::Serve {
            config,
            listen,
            token,
            quiet,
        } => serve(config, listen, token, quiet).await?,
        Command::Doctor => vkdg_cli::commands::doctor::run().await?,
        Command::Config {
            sub: ConfigSub::Check { path },
        } => {
            vkdg_cli::commands::config_check::run(&path).await?;
            // `config check` must refuse what `serve` would refuse: a provider
            // with no adapter (e.g. a typo, or a plugin that is not installed).
            let snap = load_and_validate(&path, 0)?;
            unknown_providers(&snap, &build_provider_registry()).map_err(anyhow::Error::msg)?;
        }
        Command::Config {
            sub: ConfigSub::Explain { model, json },
        } => {
            vkdg_cli::commands::config_explain::run(&model, json).await?;
        }
        Command::Request {
            sub: RequestSub::Explain { id, json },
        } => {
            vkdg_cli::commands::explain::run(&id, json).await?;
        }
        Command::Replay {
            fixture,
            base_url,
            self_test,
        } => {
            vkdg_cli::commands::replay::run(&fixture, base_url.as_deref(), self_test).await?;
        }
        Command::Setup => cmd_setup()?,
        Command::Install { config, user } => cmd_install(&config, user)?,
        Command::Update { yes, version } => cmd_update(yes, version.as_deref())?,
        Command::Plugin { sub } => handle_plugin(sub),
        Command::Login {
            provider,
            method,
            opts,
            code,
            list_methods,
        } => cmd_login(&provider, method.as_deref(), &opts, code, list_methods).await?,
        Command::Accounts { sub } => cmd_accounts(sub)?,
        Command::Keys { sub } => cmd_keys(sub)?,
        Command::Admin { sub } => cmd_admin(sub)?,
    }
    Ok(())
}

// ── Serve ─────────────────────────────────────────────────────────────────────

async fn serve(
    config_path: Option<String>,
    listen: String,
    token_override: Option<String>,
    quiet: bool,
) -> Result<()> {
    // --quiet: suppress INFO logs; RUST_LOG always wins if explicitly set.
    // Pass through ObserveConfig rather than mutating env (no unsafe needed).
    let log_level = if quiet && std::env::var("RUST_LOG").is_err() {
        "warn".to_string()
    } else {
        std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string())
    };
    let _ = init_tracing(&ObserveConfig {
        otlp_endpoint: None,
        log_level,
        ..Default::default()
    });

    let server_config = ServerConfig {
        listen_addr: listen.clone(),
        ..Default::default()
    };

    let max_concurrent: usize = std::env::var("VKDG_MAX_CONCURRENT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);

    let registry = build_provider_registry();
    let account_store = match AccountStore::open(&AccountStore::default_path()) {
        Ok(store) => Some(Arc::new(store)),
        Err(e) => {
            tracing::warn!(error = %e, "account store unavailable — `type: account` connections will fail");
            None
        }
    };
    let credentials = Arc::new(build_credentials(account_store.as_ref(), &registry));

    let mut admin_config_rx: Option<vkdg_config::ConfigRx> = None;
    // Next to accounts.db and keys.db, so history survives a restart or deploy.
    let request_log_path = AccountStore::default_path().with_file_name("requests.db");
    let request_log = match vkdg_admin::handlers::requests::RequestLog::open(
        &request_log_path,
        vkdg_admin::handlers::requests::DEFAULT_RETAIN,
    ) {
        Ok(log) => log,
        Err(e) => {
            tracing::warn!(error = %e, "request log unavailable on disk; keeping it in memory");
            vkdg_admin::handlers::requests::RequestLog::new()
        }
    };

    // Auth plugins, loaded once for both builders so a gateway started without
    // a config file has them too; plugin reloads swap this same registry.
    let hooks = Arc::new(build_hook_registry(&PluginStore::from_env()));
    let mut pipeline = if let Some(path) = &config_path {
        match load_and_validate(path, 1) {
            Ok(snap) => {
                tracing::info!(path = %path, version = snap.version, "loaded config from file");
                let mut pipeline = build_pipeline_from_snapshot(
                    &snap,
                    max_concurrent,
                    Arc::clone(&credentials),
                    Arc::clone(&registry),
                );
                pipeline.hooks = Arc::clone(&hooks);
                // Refuse to start on a route naming a hook that is not installed,
                // exactly as a reload with one is refused.
                unknown_hooks(&snap, &pipeline.hooks).map_err(anyhow::Error::msg)?;
                unknown_providers(&snap, &pipeline.provider_registry)
                    .map_err(anyhow::Error::msg)?;
                // Install hot-reload watcher; errors on bad reloads are logged, not fatal.
                let (tx, _rx) = vkdg_config::config_channel(snap);
                admin_config_rx = Some(tx.subscribe());
                spawn_config_applier(tx.subscribe(), &pipeline);
                drop(vkdg_config::watch(path.clone(), tx, 1));
                Some(pipeline)
            }
            // The operator asked for this file. Serving a different gateway (env
            // routes, maybe an ANTHROPIC_API_KEY passthrough) would hide the
            // mistake, so refuse to start, as `vkdg config check` would.
            Err(e) => anyhow::bail!("config file {path} is invalid: {e}"),
        }
    } else {
        build_pipeline_from_env(
            max_concurrent,
            Arc::clone(&credentials),
            Arc::clone(&registry),
        )
    };
    // Share request_log Arc between pipeline and admin API.
    if let Some(p) = &mut pipeline {
        p.request_log = Some(Arc::clone(&request_log));
        p.hooks = Arc::clone(&hooks);
    }

    // Combos: one service for both builders (config file or env), shared by the
    // pipeline (policies) and its router (routes) and the admin API (edits).
    // A corrupt combos.json stops startup: serving without its combos would
    // quietly send their traffic elsewhere.
    let combos = match &mut pipeline {
        Some(p) => {
            let path = AccountStore::default_path().with_file_name("combos.json");
            let resolver = Arc::new(vkdg_combos::ComboResolver::new(vec![]));
            let svc = vkdg_combos::ComboService::open(
                vkdg_combos::ComboStore::new(path),
                Arc::clone(&resolver),
                Some(Arc::clone(&p.router)),
            )
            .map_err(|e| anyhow::anyhow!("cannot load combos: {e}"))?;
            p.combo_resolver = Some(resolver);
            Some(Arc::new(svc))
        }
        None => None,
    };

    // Extract catalog for admin API before pipeline is moved into AppState.
    let admin_catalog = pipeline.as_ref().map(|p| Arc::clone(&p.catalog));

    // Plugin install/removal in the admin API rebuilds the same registries the
    // pipeline, credential refresh and config reload read, so it applies
    // without a restart.
    let reload_plugins: Option<vkdg_admin::router::PluginReload> = pipeline.as_ref().map(|p| {
        let providers = Arc::clone(&p.provider_registry);
        let hooks = Arc::clone(&p.hooks);
        Arc::new(move || reload_plugins(&PluginStore::from_env(), &providers, &hooks))
            as vkdg_admin::router::PluginReload
    });

    // Build Arc<PipelineState> now (before pipeline is moved into AppState)
    // so both the data plane and the connection-test closure share the same instance.
    let pipeline_arc: Option<Arc<PipelineState>> = pipeline.map(Arc::new);

    let connection_tester: Option<vkdg_admin::router::ConnectionTester> =
        pipeline_arc.as_ref().map(|p| {
            let p = Arc::clone(p);
            Arc::new(
                move |id: String| -> std::pin::Pin<
                    Box<
                        dyn std::future::Future<Output = vkdg_admin::router::ConnectionTestResult>
                            + Send,
                    >,
                > {
                    let p = Arc::clone(&p);
                    Box::pin(async move { test_connection_smoke(&p, &id).await })
                },
            ) as vkdg_admin::router::ConnectionTester
        });

    let mut state = AppState::new(server_config.clone());
    if let Some(p) = pipeline_arc {
        state = state.with_pipeline(p);
    }

    // Build router here (not via vkdg_http::build_router) so we can add all
    // routes before calling .with_state() once. This avoids the Router<S> type
    // mismatch that comes from calling .route() on an already-resolved Router<()>.
    let key_store = Arc::new(open_key_store()?);
    let data_auth = data_auth_from_env(Arc::clone(&key_store))?;
    let router: Router = Router::new()
        .route(
            "/v1/messages",
            post(vkdg_ingress_anthropic::handle_messages),
        )
        .route(
            "/v1/chat/completions",
            post(vkdg_ingress_openai::handle_chat_completions),
        )
        .route(
            "/v1/images/generations",
            post(vkdg_ingress_openai::handle_image_generations),
        )
        .route("/v1/models", get(vkdg_http::models::list_models))
        // Applies to the /v1 routes above only; /health and /info stay public.
        .route_layer(axum::middleware::from_fn_with_state(
            data_auth,
            vkdg_http::require_api_key,
        ))
        .route("/health", get(health))
        .route("/vkdg/v1/info", get(info))
        .route("/mcp", get(vkdg_http::mcp_discovery))
        .with_state(state);

    // ── Admin API on a separate port ───────────────────────────────────────────
    let admin_addr = std::env::var("VKDG_ADMIN_ADDR").unwrap_or_else(|_| "127.0.0.1:9090".into());
    let (bootstrap_token, auto_token): (String, Option<String>) = if let Some(t) = token_override {
        (t, None) // explicit --token: don't show it (user already knows it)
    } else {
        match std::env::var("VKDG_BOOTSTRAP_TOKEN") {
            Ok(t) => (t, None),
            Err(_) => {
                let t = uuid::Uuid::new_v4().to_string();
                (t.clone(), Some(t))
            }
        }
    };
    let config_rx = admin_config_rx.unwrap_or_else(|| {
        let (_tx, rx) = vkdg_config::config_channel(vkdg_config::ConfigSnapshot::default_empty());
        rx
    });
    let admin_state = vkdg_admin::AdminState {
        sessions: vkdg_admin::session::SessionStore::with_password_file(
            bootstrap_token,
            admin_password_path(),
            trusted_proxies_from_env()?,
        ),
        config_rx,
        started_at: std::sync::Arc::new(std::time::Instant::now()),
        key_store: Arc::clone(&key_store),
        request_log: Arc::clone(&request_log),
        combos,
        reload_plugins,
        catalog: admin_catalog,
        logins: account_store.map(|store| {
            vkdg_admin::handlers::oauth::LoginService::new(
                Arc::clone(&registry),
                store,
                Some(Arc::clone(&credentials)),
            )
        }),
        connection_tester,
    };
    // Console SPA served as fallback on the admin port (9090).
    // Data port (8080) = pure AI API. Admin port (9090) = admin API + embedded console.
    let admin_router =
        vkdg_admin::build_admin_router(admin_state).fallback_service(axum_embed::ServeEmbed::<
            ConsoleAssets,
        >::with_parameters(
            Some("index.html".to_string()),
            axum_embed::FallbackBehavior::Ok,
            Some("index.html".to_string()),
        ));
    let admin_addr_banner = admin_addr.clone();
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(&admin_addr)
            .await
            .expect("bind admin listener");
        tracing::info!(addr = %admin_addr, "admin API listening");
        // Connect info lets sign-in throttling see the real client address.
        axum::serve(
            listener,
            admin_router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .ok();
    });

    print_startup_banner(&listen, &admin_addr_banner, auto_token.as_deref());
    tracing::info!(addr = %listen, "vkdg starting");

    vkdg_http::serve(server_config, router).await
}

fn print_startup_banner(gateway_addr: &str, admin_addr: &str, auto_token: Option<&str>) {
    use std::io::IsTerminal;

    let color = std::env::var("NO_COLOR").is_err() && std::io::stdout().is_terminal();
    let (teal, bold, dim, yellow, reset) = if color {
        ("\x1b[36m", "\x1b[1m", "\x1b[2m", "\x1b[33m", "\x1b[0m")
    } else {
        ("", "", "", "", "")
    };

    fn normalize(addr: &str) -> String {
        addr.replace("0.0.0.0:", "localhost:")
            .replace("127.0.0.1:", "localhost:")
    }

    let gw = format!("http://{}", normalize(gateway_addr));
    let con = format!("http://{}", normalize(admin_addr));

    const W: usize = 54;
    let top = format!("╔{}╗", "═".repeat(W));
    let mid = format!("╠{}╣", "═".repeat(W));
    let bot = format!("╚{}╝", "═".repeat(W));
    let empty = format!("║{}║", " ".repeat(W));

    // Pad visible content to W, then wrap with colored border chars.
    let pad = |s: &str| " ".repeat(W.saturating_sub(s.chars().count()));

    let title = "   ▶  VKDG  v0.1.0";
    let gw_vis = format!("   Gateway   {}", gw);
    let con_vis = format!("   Console   {}", con);

    println!();
    println!("  {teal}{top}{reset}");
    println!("  {teal}{empty}{reset}");
    println!(
        "  {teal}║{reset}{bold}{title}{reset}{teal}{}║{reset}",
        pad(title)
    );
    println!("  {teal}{empty}{reset}");
    println!("  {teal}{mid}{reset}");
    println!("  {teal}{empty}{reset}");
    println!("  {teal}║{reset}{gw_vis}{teal}{}║{reset}", pad(&gw_vis));
    println!("  {teal}║{reset}{con_vis}{teal}{}║{reset}", pad(&con_vis));
    println!("  {teal}{empty}{reset}");

    if let Some(token) = auto_token {
        let tok_label = "   Bootstrap token (use once to sign in):";
        let tok_val = format!("   {}", token);
        println!("  {teal}{mid}{reset}");
        println!("  {teal}{empty}{reset}");
        println!(
            "  {teal}║{reset}{dim}{tok_label}{reset}{teal}{}║{reset}",
            pad(tok_label)
        );
        println!(
            "  {teal}║{reset}{yellow}{tok_val}{reset}{teal}{}║{reset}",
            pad(&tok_val)
        );
        println!("  {teal}{empty}{reset}");
    }

    println!("  {teal}{bot}{reset}");
    println!();
}

async fn health() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({"status": "ok"}))
}

async fn info() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({"version": "0.1.0"}))
}

// ── Pipeline builder — from ConfigSnapshot ────────────────────────────────────

fn build_pipeline_from_snapshot(
    snap: &ConfigSnapshot,
    max_concurrent: usize,
    credentials: Arc<CredentialManager>,
    provider_registry: Arc<ProviderRegistry>,
) -> PipelineState {
    let admission = Arc::new(AdmissionGuard::new(
        snap.limits
            .max_concurrent_requests
            .unwrap_or(max_concurrent),
    ));
    let router = Arc::new(VkdgRouter::new(snap.routes.as_ref().clone()));
    let catalog = Arc::new(ConnectionCatalog::new(snap.connections.as_ref().clone()));
    PipelineState {
        admission,
        router,
        catalog,
        credentials,
        http_client: Arc::new(HttpClient::new()),
        exporter: Arc::new(DecisionRecordExporter::new()),
        provider_registry,
        cache: None,
        combo_resolver: None,
        compressor: None,
        dedup_table: None,
        session_registry: None,
        quota_tracker: None,
        global_system_prompt: snap.gateway.global_system_prompt.clone(),
        ip_policy: Some(Arc::new(vkdg_core::net::IpPolicy::new(
            snap.ip_rules.as_ref().clone(),
        ))),
        // Feeds lowest_latency / power_of_two_choices; without it they have no data.
        latency_tracker: Some(vkdg_connections::LatencyTracker::new()),
        memory_store: None,
        eval_enabled: false,
        relay_enabled: false,
        request_log: None,
        hooks: Default::default(),
    }
}

// ── Pipeline builder — from env vars (fallback) ───────────────────────────────

fn build_pipeline_from_env(
    max_concurrent: usize,
    credentials: Arc<CredentialManager>,
    provider_registry: Arc<ProviderRegistry>,
) -> Option<PipelineState> {
    let api_key_var = "ANTHROPIC_API_KEY";
    if std::env::var(api_key_var).is_err() {
        eprintln!("ANTHROPIC_API_KEY not set — pipeline disabled, /v1/messages returns 501");
        return None;
    }

    let conn_id = ConnectionId("anthropic-default".into());

    let config = ConnectionConfig {
        id: conn_id.clone(),
        provider: ProviderKind::Anthropic,
        auth: AuthKind::ApiKey {
            env_var: api_key_var.into(),
        },
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
        credentials,
        http_client: Arc::new(HttpClient::new()),
        exporter: Arc::new(DecisionRecordExporter::new()),
        provider_registry,
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
        relay_enabled: false,
        request_log: None,
        hooks: Default::default(),
    })
}

fn build_provider_registry() -> Arc<ProviderRegistry> {
    Arc::new(load_provider_registry(&PluginStore::from_env()))
}

/// Built-ins first, then installed WASM providers. Called at startup and on
/// every plugin install/removal.
fn load_provider_registry(store: &PluginStore) -> ProviderRegistry {
    let mut r = ProviderRegistry::empty();
    r.register(Arc::new(AnthropicAdapter));
    r.register(Arc::new(OpenAIAdapter));
    r.register(Arc::new(gemini_provider()));
    r.register(Arc::new(groq_provider()));
    r.register(Arc::new(together_provider()));
    r.register(Arc::new(fireworks_provider()));
    r.register(Arc::new(deepseek_provider()));
    r.register(Arc::new(mistral_provider()));
    r.register(Arc::new(ClaudeCodeAdapter));
    r.register(Arc::new(CodexAdapter));
    r.register(Arc::new(KiroAdapter));
    r.register(Arc::new(KimiCodingAdapter));
    r.register(Arc::new(AntigravityAdapter));
    r.register(Arc::new(GitHubCopilotAdapter));
    r.register(Arc::new(sambanova_provider()));
    r.register(Arc::new(cerebras_provider()));
    r.register(Arc::new(nvidia_nim_provider()));
    if e2e_fake_oauth::enabled() {
        r.register(Arc::new(e2e_fake_oauth::FakeOAuth::default()));
    }
    // Installed WASM providers register after the built-ins and may only take
    // a free id: an adapter receives the credentials of every connection that
    // names it, so a plugin must never replace `anthropic` or `kiro`.
    for (name, loaded) in store.load_providers() {
        let adapter = match loaded {
            Ok(adapter) => adapter,
            Err(e) => {
                tracing::error!(plugin = %name, error = %e, "WASM provider plugin failed to load; skipped");
                continue;
            }
        };
        let provider = vkdg_provider_sdk::ProviderAdapter::id(&adapter).to_owned();
        match r.try_register(Arc::new(adapter)) {
            Ok(()) => tracing::info!(plugin = %name, %provider, "loaded WASM provider plugin"),
            Err(e) => tracing::warn!(plugin = %name, error = %e, "WASM provider plugin refused"),
        }
    }
    r
}

/// Credential manager with account auth enabled when the store opened.
fn build_credentials(
    store: Option<&Arc<AccountStore>>,
    registry: &Arc<ProviderRegistry>,
) -> CredentialManager {
    let mgr = CredentialManager::new();
    match store {
        Some(store) => mgr.with_accounts(Arc::clone(store), Arc::clone(registry) as _),
        None => mgr,
    }
}

// ── Login / accounts ──────────────────────────────────────────────────────────

fn parse_opts(opts: &[String]) -> Result<LoginParams> {
    opts.iter()
        .map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.trim().to_owned(), v.to_owned()))
                .ok_or_else(|| anyhow::anyhow!("--opt expects KEY=VALUE, got '{kv}'"))
        })
        .collect()
}

async fn cmd_login(
    provider: &str,
    method: Option<&str>,
    opts: &[String],
    code: Option<String>,
    list_methods: bool,
) -> Result<()> {
    let registry = build_provider_registry();
    let adapter = registry
        .get(provider)
        .ok_or_else(|| anyhow::anyhow!("unknown provider '{provider}'"))?;
    let oauth = adapter
        .oauth()
        .ok_or_else(|| anyhow::anyhow!("provider '{provider}' does not support login"))?;

    if list_methods {
        for m in oauth.login_methods() {
            println!("{}  ({:?}) {}", m.id, m.flow, m.label);
            for f in &m.fields {
                let req = if f.required { "required" } else { "optional" };
                let def = f
                    .default
                    .as_deref()
                    .map(|d| format!(" [default: {d}]"))
                    .unwrap_or_default();
                println!("    --opt {}=...  {} ({req}){def}", f.id, f.label);
            }
        }
        return Ok(());
    }

    let method = find_login_method(oauth, method)?;
    let params = resolve_login_params(&method, &parse_opts(opts)?)?;

    let result: LoginResult = match method.flow {
        OAuthFlow::DeviceCode => {
            let auth = oauth.start_device_login(&method.id, &params).await?;
            println!(
                "Open: {}",
                auth.verification_uri_complete
                    .as_deref()
                    .unwrap_or(&auth.verification_uri)
            );
            println!("Code: {}", auth.user_code);
            println!(
                "Waiting for approval (expires in {}s)…",
                auth.expires_in_secs
            );
            run_device_login(oauth, &method.id, &auth, tokio::time::sleep).await?
        }
        OAuthFlow::AuthorizationCodePkce => {
            let auth = oauth.start_pkce_login(&method.id, &params).await?;
            println!("Open: {}", auth.authorize_url);
            let code = match code {
                Some(c) => c,
                None => {
                    println!("Paste the authorization code or callback URL:");
                    let mut line = String::new();
                    std::io::stdin().read_line(&mut line)?;
                    line.trim().to_owned()
                }
            };
            oauth
                .finish_pkce_login(&method.id, &auth.state, &code)
                .await?
        }
        OAuthFlow::ImportToken => oauth.import_token(&method.id, &params).await?,
    };

    let label = if result.label.is_empty() {
        provider.to_owned()
    } else {
        result.label.clone()
    };
    let account = Account::from_token_pair(provider, &label, result.tokens);
    let path = AccountStore::default_path();
    AccountStore::open(&path)?.upsert(&account)?;

    println!(
        "Saved account {} ({label}) to {}",
        account.id,
        path.display()
    );
    println!("\nReference it from your config:\n");
    println!("connections:\n  - id: {provider}\n    provider: {provider}\n    auth: {{ type: account, account: {} }}\n    models: [\"<model>\"]", account.id);
    Ok(())
}

fn cmd_accounts(sub: AccountsSub) -> Result<()> {
    let store = AccountStore::open(&AccountStore::default_path())?;
    match sub {
        AccountsSub::List => {
            let accounts = store.list()?;
            if accounts.is_empty() {
                println!("No accounts. Run `vkdg login <provider>`.");
            }
            for a in accounts {
                let exp = a
                    .expires_at
                    .map_or_else(|| "unknown".to_owned(), |t| t.to_rfc3339());
                println!("{}\t{}\t{}\texpires {exp}", a.id, a.provider, a.label);
            }
        }
        AccountsSub::Remove { id } => {
            if store.remove(&id)? {
                println!("Removed {id}");
            } else {
                anyhow::bail!("account '{id}' not found");
            }
        }
    }
    Ok(())
}

// ── Config hot reload ─────────────────────────────────────────────────────────

/// Apply every validated config snapshot to the live pipeline.
///
/// Only config-derived parts are swapped: the route table, connection configs,
/// and IP rules. Live state (in-flight counters, health, credentials, key store,
/// trackers) is kept, so a reload never resets capacity accounting. The
/// watcher already rejects invalid files, so this only ever sees valid ones.
fn spawn_config_applier(mut rx: vkdg_config::ConfigRx, pipeline: &PipelineState) {
    let hooks = Arc::clone(&pipeline.hooks);
    let providers = Arc::clone(&pipeline.provider_registry);
    let router = Arc::clone(&pipeline.router);
    let catalog = Arc::clone(&pipeline.catalog);
    let ip_policy = pipeline.ip_policy.clone();
    // The first value is the startup snapshot, already applied.
    rx.mark_unchanged();
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let snap = Arc::clone(&rx.borrow_and_update());
            // Invariant 7: a reload applies whole or not at all. A route naming
            // a missing hook rejects the entire snapshot before anything moves.
            let checked =
                unknown_hooks(&snap, &hooks).and_then(|()| unknown_providers(&snap, &providers));
            if let Err(e) = checked {
                tracing::warn!(version = snap.version, error = %e, "config reload rejected — keeping current snapshot");
                continue;
            }
            catalog.apply(snap.connections.as_ref().clone()).await;
            router.replace_routes(snap.routes.as_ref().clone());
            if let Some(policy) = &ip_policy {
                policy.replace(snap.ip_rules.as_ref().clone());
            }
            tracing::info!(version = snap.version, "config applied to data plane");
        }
    });
}

// ── Route hooks ───────────────────────────────────────────────────────────────

/// Adapts an installed auth plugin to the pipeline's hook contract. The plugin
/// gets identity fields only (see `vkdg_http::hooks::HookRequest`), never the
/// client's headers or key.
struct WasmAuthHook(vkdg_plugin_host::WasmAuth);

impl vkdg_http::hooks::AuthHook for WasmAuthHook {
    fn check(&self, request: &vkdg_http::hooks::HookRequest) -> vkdg_http::hooks::HookVerdict {
        use vkdg_http::hooks::HookVerdict;
        use vkdg_plugin_host::{AuthOutcome, AuthPlugin};
        let Ok(json) = serde_json::to_string(request) else {
            return HookVerdict::Deny("could not serialise hook request".into());
        };
        match self.0.authenticate(&json) {
            AuthOutcome::Allowed(_) => HookVerdict::Allow,
            AuthOutcome::Denied(reason) => HookVerdict::Deny(reason),
        }
    }
}

/// Auth plugins installed on this gateway, by plugin name. Broken ones are
/// logged and left out, so routes naming them are refused at load.
fn build_hook_registry(store: &PluginStore) -> vkdg_http::hooks::HookRegistry {
    let mut reg = vkdg_http::hooks::HookRegistry::default();
    for (name, loaded) in store.load_auth() {
        match loaded {
            Ok(auth) => {
                tracing::info!(plugin = %name, "loaded WASM auth plugin");
                reg.register_auth(name, Arc::new(WasmAuthHook(auth)));
            }
            Err(e) => {
                tracing::error!(plugin = %name, error = %e, "WASM auth plugin failed to load; skipped")
            }
        }
    }
    reg
}

/// Rebuild the live registries from `store` in place. Every holder of these
/// `Arc`s (pipeline, credential refresh, config reload) sees the new set.
fn reload_plugins(
    store: &PluginStore,
    providers: &ProviderRegistry,
    hooks: &vkdg_http::hooks::HookRegistry,
) {
    providers.replace(load_provider_registry(store));
    hooks.replace(build_hook_registry(store));
    tracing::info!(dir = %store.root().display(), "plugins reloaded");
}

/// Every route hook id must name a loaded plugin.
fn unknown_hooks(
    snap: &ConfigSnapshot,
    hooks: &vkdg_http::hooks::HookRegistry,
) -> Result<(), String> {
    for route in snap.routes.iter() {
        for id in &route.plugin_hooks.auth {
            if !hooks.has_auth(id) {
                return Err(format!(
                    "routes[{}].hooks.auth: plugin {id:?} is not an installed auth plugin",
                    route.id.0
                ));
            }
        }
    }
    Ok(())
}

/// Every connection must name a provider with a registered adapter. Without
/// this, `provider: openai-compatt` loads fine and fails on the first request.
fn unknown_providers(snap: &ConfigSnapshot, registry: &ProviderRegistry) -> Result<(), String> {
    for conn in snap.connections.iter() {
        let id = conn.provider.adapter_id();
        if registry.get(id).is_none() {
            let mut known = registry.ids();
            known.sort_unstable();
            return Err(format!(
                "connection '{}': provider {id:?} is not a built-in provider or an installed plugin \
                 (known: {}; or openai-compat / anthropic-compat with base_url)",
                conn.id.0,
                known.join(", ")
            ));
        }
    }
    Ok(())
}

// ── Admin console password ────────────────────────────────────────────────────

/// `$VKDG_ADMIN_PASSWORD_FILE`, else `admin.password` next to `accounts.db`, so
/// the password lives on the same persistent volume.
fn admin_password_path() -> std::path::PathBuf {
    std::env::var_os("VKDG_ADMIN_PASSWORD_FILE").map_or_else(
        || AccountStore::default_path().with_file_name("admin.password"),
        Into::into,
    )
}

fn cmd_admin(sub: AdminSub) -> Result<()> {
    match sub {
        AdminSub::SetPassword => {
            use std::io::{BufRead, IsTerminal};
            let stdin = std::io::stdin();
            let password = if stdin.is_terminal() {
                let first = inquire::Password::new("New console password:")
                    .without_confirmation()
                    .prompt()?;
                let again = inquire::Password::new("Repeat it:")
                    .without_confirmation()
                    .prompt()?;
                anyhow::ensure!(first == again, "the passwords do not match");
                first
            } else {
                let mut line = String::new();
                stdin.lock().read_line(&mut line)?;
                line.trim_end_matches(['\r', '\n']).to_owned()
            };
            let store = vkdg_admin::session::SessionStore::with_password_file(
                String::new(),
                admin_password_path(),
                vec![],
            );
            store.set_password(&password).map_err(anyhow::Error::msg)?;
            println!(
                "Console password set in {}. Running gateways use it at the next sign-in.",
                admin_password_path().display()
            );
        }
    }
    Ok(())
}

// ── Data-plane API keys ───────────────────────────────────────────────────────

/// `$VKDG_KEYS_DB`, else `keys.db` next to the account store, so one mounted
/// volume holds both.
fn key_store_path() -> std::path::PathBuf {
    std::env::var_os("VKDG_KEYS_DB").map_or_else(
        || AccountStore::default_path().with_file_name("keys.db"),
        Into::into,
    )
}

fn open_key_store() -> Result<vkdg_governance::VirtualKeyStore> {
    let path = key_store_path();
    vkdg_governance::VirtualKeyStore::open(&path)
        .map_err(|e| anyhow::anyhow!("cannot open API key store {}: {e}", path.display()))
}

/// Keys are required on every bind address. A loopback listener behind a
/// reverse proxy is still public, which is exactly how an open relay happens.
/// `VKDG_DATA_AUTH=off` is the only way out, and it is loud.
fn data_auth_from_env(store: Arc<vkdg_governance::VirtualKeyStore>) -> Result<vkdg_http::DataAuth> {
    let auth = match std::env::var("VKDG_DATA_AUTH").as_deref() {
        Ok("off") => {
            tracing::warn!(
                "VKDG_DATA_AUTH=off: /v1/* accepts requests WITHOUT an API key. \
                 Anyone who can reach this port spends your provider accounts."
            );
            vkdg_http::DataAuth::disabled()
        }
        _ => {
            let active = store
                .list()
                .map(|keys| keys.iter().filter(|k| !k.is_revoked()).count())
                .unwrap_or(0);
            if active == 0 {
                tracing::warn!(
                    path = %key_store_path().display(),
                    "no API keys yet: /v1/* rejects every request until you run `vkdg keys create <name>`"
                );
            }
            vkdg_http::DataAuth::required(store)
        }
    }
    .with_trusted_proxies(trusted_proxies_from_env()?);
    Ok(auth)
}

/// `VKDG_TRUSTED_PROXIES`: comma-separated proxies whose `X-Forwarded-For` is
/// believed, e.g. `127.0.0.1,172.16.0.0/12` for a local nginx or Docker bridge.
/// Empty (the default) means the socket address is the client. An invalid
/// entry stops startup: a typo here would silently trust nobody, or everybody.
fn trusted_proxies_from_env() -> Result<Vec<vkdg_core::net::IpNet>> {
    let raw = std::env::var("VKDG_TRUSTED_PROXIES").unwrap_or_default();
    let entries: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    vkdg_core::net::parse_ip_list("VKDG_TRUSTED_PROXIES", &entries).map_err(anyhow::Error::msg)
}

fn cmd_keys(sub: KeysSub) -> Result<()> {
    use vkdg_governance::{KeyScope, VirtualKeyId};
    let store = open_key_store()?;
    match sub {
        KeysSub::Create {
            name,
            tenant,
            inference_only,
            models,
            ips,
            expires_in_days,
            monthly_tokens,
            rpm,
            no_log,
        } => {
            let scopes = if inference_only {
                vec![KeyScope::DataInference]
            } else {
                KeyScope::DEFAULT.to_vec()
            };
            let (key, raw) = store.create(vkdg_governance::NewKey {
                name,
                tenant_id: tenant,
                scopes,
                expires_at: expires_in_days
                    .map(|d| chrono::Utc::now() + chrono::Duration::days(i64::from(d))),
                allowed_models: models,
                allowed_ips: vkdg_core::net::parse_ip_list("--ip", &ips)
                    .map_err(anyhow::Error::msg)?,
                monthly_token_limit: monthly_tokens,
                requests_per_minute: rpm,
                no_log,
            })?;
            // stdout carries only the secret so `$(vkdg keys create ci)` works;
            // the context goes to stderr.
            eprintln!(
                "Created key {} ({}). It will not be shown again.",
                key.id.0, key.name
            );
            println!("{raw}");
        }
        KeysSub::List => {
            let keys = store.list()?;
            if keys.is_empty() {
                println!("No keys. Run `vkdg keys create <name>`.");
            }
            for k in keys {
                let state = if k.is_revoked() { "revoked" } else { "active" };
                let used = k
                    .last_used_at
                    .map_or_else(|| "never".to_owned(), |t| t.to_rfc3339());
                println!(
                    "{}\t{}\t{}…\t{state}\tlast used {used}",
                    k.id.0, k.name, k.prefix
                );
            }
        }
        KeysSub::Revoke { id } => {
            if store.revoke(&VirtualKeyId(id.clone()))? {
                println!("Revoked {id}");
            } else {
                anyhow::bail!("key '{id}' not found");
            }
        }
    }
    Ok(())
}

// ── Setup wizard ──────────────────────────────────────────────────────────────

fn cmd_setup() -> Result<()> {
    use console::style;
    use indicatif::{ProgressBar, ProgressStyle};
    use inquire::{
        ui::{Color, RenderConfig, Styled},
        Select, Text,
    };
    use std::time::Duration;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn spinner(msg: &str) -> ProgressBar {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::default_spinner()
                .tick_strings(&["◒", "◐", "◓", "◑", "◇"])
                .template("{spinner:.magenta}  {msg}")
                .unwrap(),
        );
        pb.set_message(msg.to_owned());
        pb.enable_steady_tick(Duration::from_millis(80));
        pb
    }

    fn ok(msg: &str) {
        eprintln!("{}  {}", console::style(STEP_DONE).green(), msg);
    }

    fn warn(msg: &str) {
        eprintln!("{}  {}", console::style(STEP_ERR).yellow(), msg);
    }

    fn clack_theme() -> RenderConfig<'static> {
        RenderConfig::default()
            .with_prompt_prefix(Styled::new("◆").with_fg(Color::LightCyan))
            .with_answered_prompt_prefix(Styled::new("◇").with_fg(Color::LightGreen))
    }

    let bar = || eprintln!("{}", style(BAR).dim());

    // ── intro ─────────────────────────────────────────────────────────────────
    eprintln!("{} {}", style("┌").dim(), style("vkdg setup").bold());
    eprintln!("{}  Set up your AI gateway.", style("│").dim());
    bar();

    // ── Step 1: provider ─────────────────────────────────────────────────────
    let provider_labels = vec![
        "Anthropic  (Claude)",
        "OpenAI  (GPT / o-series)",
        "Groq  (free tier available)",
        "Google Gemini",
        "DeepSeek",
        "Mistral",
        "Together AI",
        "Fireworks AI",
        "Kiro  (Amazon Q)",
        "Custom  (any OpenAI-compatible endpoint)",
    ];

    let provider_choice = Select::new("Provider:", provider_labels)
        .with_render_config(clack_theme())
        .prompt()?;
    bar();

    // ── Step 2: base URL for custom ───────────────────────────────────────────
    let is_custom = provider_choice.starts_with("Custom");
    let base_url_opt: Option<String> = if is_custom {
        let url = Text::new("Base URL:")
            .with_placeholder("http://localhost:11434")
            .with_render_config(clack_theme())
            .prompt()?;
        bar();
        Some(url)
    } else {
        None
    };

    let (provider_id, default_env, model_pattern, display_name) = match provider_choice
        .split_whitespace()
        .next()
        .unwrap_or("anthropic")
    {
        "Anthropic" => ("anthropic", "ANTHROPIC_API_KEY", "claude-*", "Anthropic"),
        "OpenAI" => ("openai", "OPENAI_API_KEY", "gpt-*", "OpenAI"),
        "Groq" => ("groq", "GROQ_API_KEY", "llama-*", "Groq"),
        "Google" => ("gemini", "GEMINI_API_KEY", "gemini-*", "Gemini"),
        "DeepSeek" => ("deepseek", "DEEPSEEK_API_KEY", "deepseek-*", "DeepSeek"),
        "Mistral" => ("mistral", "MISTRAL_API_KEY", "mistral-*", "Mistral"),
        "Together" => (
            "together",
            "TOGETHER_API_KEY",
            "meta-llama/*",
            "Together AI",
        ),
        "Fireworks" => (
            "fireworks",
            "FIREWORKS_API_KEY",
            "accounts/*",
            "Fireworks AI",
        ),
        "Kiro" => (
            "kiro",
            "KIRO_API_KEY",
            "claude-*,gpt-5.6-*,minimax-*,deepseek-*,glm-*,qwen3-*,auto",
            "Kiro / Amazon Q",
        ),
        _ => ("openai-compat", "API_KEY", "*", "Custom"),
    };

    // ── Step 3: API key env var ───────────────────────────────────────────────
    let prefilled = std::env::var(default_env).ok();
    let key_hint = prefilled.as_ref().map(|k| {
        format!(
            "current: {}...{}",
            &k[..4.min(k.len())],
            &k[k.len().saturating_sub(4)..]
        )
    });

    let env_var = if let Some(hint) = &key_hint {
        eprintln!(
            "{}  {} already set ({})",
            style(BAR).dim(),
            style(default_env).dim(),
            hint
        );
        bar();
        Text::new("Environment variable name:")
            .with_default(default_env)
            .with_render_config(clack_theme())
            .prompt()?
    } else {
        Text::new("Environment variable name:")
            .with_default(default_env)
            .with_help_message("The variable holding your API key — not stored in the config file")
            .with_render_config(clack_theme())
            .prompt()?
    };
    bar();

    // ── Step 4: model ─────────────────────────────────────────────────────────
    let (model_options, default_model) = match provider_id {
        "anthropic" => (
            vec![
                "claude-sonnet-4-5  (recommended)",
                "claude-opus-4      (most capable)",
                "claude-haiku-3-5   (fastest)",
            ],
            "claude-sonnet-4-5",
        ),
        "openai" => (
            vec![
                "gpt-4o            (recommended)",
                "o3-mini           (reasoning)",
                "gpt-4o-mini       (fastest)",
            ],
            "gpt-4o",
        ),
        "groq" => (
            vec![
                "llama-3.3-70b-versatile  (recommended, free)",
                "llama-3.1-8b-instant     (fastest, free)",
            ],
            "llama-3.3-70b-versatile",
        ),
        "gemini" => (
            vec!["gemini-2.0-flash   (recommended)", "gemini-1.5-pro"],
            "gemini-2.0-flash",
        ),
        "kiro" => (
            vec![
                "auto               (recommended, provider picks)",
                "claude-opus-5.5    (most capable)",
                "claude-opus-5",
                "claude-sonnet-5",
                "claude-opus-4.8",
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "claude-opus-4.7",
                "claude-opus-4.6",
                "claude-sonnet-4.6",
                "claude-opus-4.5",
                "claude-sonnet-4.5",
                "claude-sonnet-4",
                "claude-haiku-4.5   (fastest)",
                "deepseek-3.2",
                "minimax-m2.5",
                "minimax-m2.1",
                "glm-5",
                "qwen3-coder-next",
            ],
            "auto",
        ),
        _ => (vec!["(any model — set per-request)"], "*"),
    };

    let model_choice = if model_options.len() > 1 {
        let choice = Select::new("Default model:", model_options)
            .with_render_config(clack_theme())
            .prompt()?;
        bar();
        choice
            .split_whitespace()
            .next()
            .unwrap_or(default_model)
            .to_string()
    } else {
        bar();
        default_model.to_string()
    };

    // ── Step 5: config path ───────────────────────────────────────────────────
    let config_path = Text::new("Config file path:")
        .with_default(default_config_path())
        .with_render_config(clack_theme())
        .prompt()?;
    bar();

    // ── Step 6: verify API key ────────────────────────────────────────────────
    if let Ok(key_val) = std::env::var(&env_var) {
        let sp = spinner(&format!("Verifying {} key...", display_name));
        std::thread::sleep(Duration::from_millis(800));
        let looks_ok = !key_val.is_empty() && key_val.len() > 8;
        sp.finish_and_clear();
        if looks_ok {
            ok(&format!("{} key looks valid", display_name));
        } else {
            warn(&format!(
                "{} key may be invalid — proceeding anyway",
                display_name
            ));
        }
        bar();
    }

    // ── Build config ──────────────────────────────────────────────────────────
    let config_yaml = setup_yaml(
        provider_id,
        base_url_opt.as_deref(),
        &env_var,
        model_pattern,
    );

    // ── Write config ──────────────────────────────────────────────────────────
    let sp = spinner("Writing config...");
    let path = std::path::Path::new(&config_path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let write_ok = std::fs::write(path, &config_yaml).is_ok();
    sp.finish_and_clear();

    if write_ok {
        ok(&format!("Config written to {config_path}"));
    } else {
        warn(&format!("Could not write to {config_path} (permission?)"));
        eprintln!("{}  Config content:", style(BAR).dim());
        for line in config_yaml.lines() {
            eprintln!("{}    {}", style(BAR).dim(), style(line).dim());
        }
    }
    bar();

    // ── systemd on Linux ──────────────────────────────────────────────────────
    #[cfg(target_os = "linux")]
    {
        use inquire::Confirm;
        if Confirm::new("Install as a systemd service?")
            .with_default(true)
            .with_render_config(clack_theme())
            .prompt()
            .unwrap_or(false)
        {
            bar();
            let user_mode = !is_root();
            let _ = cmd_install(&config_path, user_mode);
        }
        bar();
    }

    // ── outro ─────────────────────────────────────────────────────────────────
    eprintln!(
        "{}  {}",
        style("◆").green().bold(),
        style("Setup complete").bold()
    );
    eprintln!("{}", style(BAR).dim());
    eprintln!(
        "{}  {:<12} {} — {}",
        style(BAR).dim(),
        style("Provider").dim(),
        display_name,
        model_choice
    );
    eprintln!(
        "{}  {:<12} http://localhost:8080",
        style(BAR).dim(),
        style("Gateway").dim()
    );
    eprintln!(
        "{}  {:<12} http://localhost:9090",
        style(BAR).dim(),
        style("Console").dim()
    );
    eprintln!(
        "{}  {:<12} {}",
        style(BAR).dim(),
        style("Config").dim(),
        config_path
    );
    eprintln!("{}", style(BAR).dim());
    eprintln!("{}  Start the gateway:", style(BAR).dim());
    eprintln!("{}", style(BAR).dim());
    eprintln!("{}    export {}=your-key", style(BAR).dim(), env_var);
    eprintln!(
        "{}    vkdg serve --config {}",
        style(BAR).dim(),
        config_path
    );
    eprintln!("{}", style(BAR).dim());
    eprintln!(
        "{} {}",
        style("└").dim(),
        style("Docs: https://github.com/vkdprojects/vkdg").dim()
    );
    eprintln!();

    Ok(())
}

fn default_config_path() -> &'static str {
    if cfg!(target_os = "linux") {
        "/etc/vkdg/config.yaml"
    } else {
        "~/.config/vkdg/config.yaml"
    }
}

// ── Install (systemd) ─────────────────────────────────────────────────────────

fn cmd_install(config_path: &str, user_mode: bool) -> Result<()> {
    let binary_path =
        std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("/usr/local/bin/vkdg"));

    let unit = format!(
        "[Unit]\n\
         Description=VKDG AI Gateway\n\
         After=network.target\n\
         StartLimitIntervalSec=0\n\
         \n\
         [Service]\n\
         Type=simple\n\
         Restart=always\n\
         RestartSec=5\n\
         ExecStart={} serve --config {}\n\
         EnvironmentFile=-/etc/vkdg/env\n\
         StandardOutput=journal\n\
         StandardError=journal\n\
         SyslogIdentifier=vkdg\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        binary_path.display(),
        config_path,
    );

    let unit_dir = if user_mode {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        std::path::PathBuf::from(home).join(".config/systemd/user")
    } else {
        std::path::PathBuf::from("/etc/systemd/system")
    };

    std::fs::create_dir_all(&unit_dir)?;
    let unit_path = unit_dir.join("vkdg.service");
    std::fs::write(&unit_path, unit)?;
    println!("✓ Systemd unit written: {}", unit_path.display());

    let systemctl_args: &[&str] = if user_mode { &["--user"] } else { &[] };
    let reload = std::process::Command::new("systemctl")
        .args(systemctl_args)
        .arg("daemon-reload")
        .status();
    if reload.is_ok() {
        let _ = std::process::Command::new("systemctl")
            .args(systemctl_args)
            .args(["enable", "--now", "vkdg"])
            .status();
        println!("✓ Service enabled and started.");
        println!(
            "  Check status: systemctl {}status vkdg",
            if user_mode { "--user " } else { "" }
        );
    }

    Ok(())
}

// ── Self-update ───────────────────────────────────────────────────────────────

fn cmd_update(no_confirm: bool, version: Option<&str>) -> Result<()> {
    println!("Checking for updates...");
    let mut builder = self_update::backends::github::Update::configure();
    builder
        .repo_owner("vkdprojects")
        .repo_name("vkdg")
        .bin_name("vkdg")
        .show_download_progress(true)
        .no_confirm(no_confirm)
        .current_version(env!("CARGO_PKG_VERSION"));
    if let Some(v) = version {
        builder.release_tag(v);
    }
    match builder.build()?.update()? {
        self_update::VersionStatus::UpToDate(v) => println!("Already up to date (v{v})."),
        self_update::VersionStatus::Updated(v) => {
            println!("\x1b[32m✓ Updated to v{v}\x1b[0m");
            println!("Restart the vkdg service to apply:");
            println!("  systemctl restart vkdg");
        }
        _ => {}
    }
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
/// Returns true if the process can write to the system-wide systemd unit directory.
fn is_root() -> bool {
    std::path::Path::new("/etc/systemd/system").exists()
        && std::fs::metadata("/etc/systemd/system")
            .map(|m| !m.permissions().readonly())
            .unwrap_or(false)
}

// ── Plugin subcommands ────────────────────────────────────────────────────────

use anyhow::Context as _;
use vkdg_plugin_host::{PluginStore, RegistryIndex, RegistryManifest, DEFAULT_REGISTRY};

fn handle_plugin(sub: PluginSub) {
    use PluginSub::*;
    match sub {
        Search { query, kind } => {
            if let Err(e) = plugin_search(query.as_deref().unwrap_or(""), kind.as_ref()) {
                eprintln!("\x1b[31m✗\x1b[0m {e}");
                std::process::exit(1);
            }
        }
        Install {
            plugin,
            registry_ref,
        } => {
            if let Err(e) = plugin_install(&plugin, &registry_ref) {
                eprintln!("\x1b[31m✗\x1b[0m {e}");
                std::process::exit(1);
            }
        }
        List => {
            let store = PluginStore::from_env();
            match store.list() {
                Err(e) => {
                    eprintln!(
                        "\x1b[31m✗\x1b[0m cannot read {}: {e}",
                        store.root().display()
                    );
                    std::process::exit(1);
                }
                Ok(listing) if listing.installed.is_empty() && listing.broken.is_empty() => {
                    println!("No plugins installed in {}.", store.root().display());
                    println!();
                    println!("  Install one with:  vkdg plugin install <name|url|path>");
                }
                Ok(listing) => {
                    println!("Installed plugins ({}):", store.root().display());
                    for b in &listing.broken {
                        println!("  \x1b[31m✗\x1b[0m {}  will not load: {}", b.dir, b.error);
                    }
                    for p in &listing.installed {
                        let m = &p.manifest;
                        println!("  {:<24} {:<10} {:?}", m.name, m.version, m.kind);
                        if !m.models.is_empty() {
                            println!("  {:<24} models: {}", "", m.models.join(", "));
                        }
                    }
                }
            }
            println!();
            println!("  Built-in providers (no install needed):");
            println!("    anthropic, openai, gemini, groq, deepseek, mistral, together, fireworks");
            println!("    claude-code, codex, kiro, kimi-coding, github-copilot, antigravity");
        }
        Remove { name } => {
            let store = PluginStore::from_env();
            match store.remove(&name) {
                Ok(()) => println!("\x1b[32m✓\x1b[0m removed {name}"),
                Err(e) => {
                    eprintln!("\x1b[31m✗\x1b[0m {e}");
                    std::process::exit(1);
                }
            }
        }
        Update => {
            // Updating means re-resolving each manifest against its source, which
            // needs the registry client; refuse clearly rather than no-op.
            let store = PluginStore::from_env();
            let installed = store.list().map(|l| l.installed).unwrap_or_default();
            if installed.is_empty() {
                println!("No plugins installed.");
            } else {
                println!("Re-install to update:");
                for p in &installed {
                    println!(
                        "  vkdg plugin install {}   (currently {})",
                        p.manifest.name, p.manifest.version
                    );
                }
            }
        }
        Tap { repo, list } => {
            if list {
                println!("Taps: (none — tap support coming in Phase 3)");
            } else if let Some(r) = repo {
                println!("Tap support coming in Phase 3. Noted: {r}");
            }
        }
        ValidateManifest { path } => match std::fs::read_to_string(&path) {
            Err(e) => {
                eprintln!("\x1b[31m✗\x1b[0m cannot read {path}: {e}");
                std::process::exit(1);
            }
            // Validate against the published schema, not just YAML syntax: this is
            // what the registry bot runs, so an author sees the same verdict locally.
            Ok(content) => match RegistryManifest::from_yaml(&content) {
                Ok(m) => {
                    println!(
                        "\x1b[32m✓\x1b[0m {path}: {} {} ({:?})",
                        m.name, m.version, m.kind
                    );
                    if !m.installs_without_rebuild() {
                        println!("  note: ships as a Rust crate, so it needs a gateway rebuild");
                    }
                }
                Err(e) => {
                    eprintln!("\x1b[31m✗\x1b[0m {path}: {e}");
                    std::process::exit(1);
                }
            },
        },
    }
}

/// Install a plugin from a local directory, a manifest file, or a URL.
///
/// Accepted forms:
/// - a directory containing `manifest.yaml` (+ `plugin.wasm` for wasm plugins)
/// - a path to a manifest `.yaml`
/// - an `https://` URL to a manifest `.yaml`
///
/// A wasm artefact is fetched from the manifest's `install.wasm` and checked
/// against its checksum before anything is written.
fn plugin_install(source: &str, registry_ref: &str) -> Result<()> {
    let store = PluginStore::from_env();
    let (manifest, local_wasm) = load_manifest(source, registry_ref)?;

    let wasm = match manifest.source_kind() {
        Ok(vkdg_plugin_host::SourceKind::Wasm) => match local_wasm {
            Some(bytes) => Some(bytes),
            None => {
                let url = manifest
                    .install
                    .wasm
                    .as_deref()
                    .context("manifest declares a wasm install without a URL")?;
                println!("Fetching {url}");
                Some(fetch_bytes(url)?)
            }
        },
        _ => None,
    };

    let installed = store
        .install(&manifest, wasm.as_deref())
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!(
        "\x1b[32m✓\x1b[0m installed {} {} to {}",
        installed.manifest.name,
        installed.manifest.version,
        installed.dir.display()
    );
    if let Some(snippet) = &installed.manifest.install.config_snippet {
        println!();
        println!("Add to your config:");
        for line in snippet.lines() {
            println!("  {line}");
        }
    } else {
        use vkdg_plugin_host::registry_manifest::PluginManifestKind as K;
        let name = &installed.manifest.name;
        println!();
        match installed.manifest.kind {
            K::Provider | K::OauthProvider => {
                println!("Use it from a connection in your config:");
                println!("  provider: {name}");
            }
            K::Auth => {
                println!("Use it from a route in your config:");
                println!("  hooks: {{ auth: [{name}] }}");
            }
            other => {
                println!(
                    "Note: {other:?} plugins are installed but this gateway does not run them yet."
                );
            }
        }
    }
    Ok(())
}

/// Resolve a source into a manifest, plus component bytes when they are already
/// local (an installed directory needs no download).
fn load_manifest(source: &str, registry_ref: &str) -> Result<(RegistryManifest, Option<Vec<u8>>)> {
    let path = std::path::Path::new(source);

    if path.is_dir() {
        let manifest_path = path.join("manifest.yaml");
        let yaml = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?;
        let manifest = RegistryManifest::from_yaml(&yaml).map_err(|e| anyhow::anyhow!("{e}"))?;
        let wasm_path = path.join("plugin.wasm");
        let wasm = wasm_path
            .is_file()
            .then(|| std::fs::read(&wasm_path))
            .transpose()?;
        return Ok((manifest, wasm));
    }
    if path.is_file() {
        let yaml =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let manifest = RegistryManifest::from_yaml(&yaml).map_err(|e| anyhow::anyhow!("{e}"))?;
        // A manifest beside its component: common when testing a local build.
        let sibling = path.with_file_name("plugin.wasm");
        let wasm = sibling
            .is_file()
            .then(|| std::fs::read(&sibling))
            .transpose()?;
        return Ok((manifest, wasm));
    }
    if source.starts_with("https://") || source.starts_with("http://") {
        let yaml = String::from_utf8(fetch_bytes(source)?)
            .context("manifest at that URL is not valid UTF-8")?;
        let manifest = RegistryManifest::from_yaml(&yaml).map_err(|e| anyhow::anyhow!("{e}"))?;
        return Ok((manifest, None));
    }

    // A bare name (optionally name@version) resolves through the registry index.
    let (name, wanted_version) = match source.split_once('@') {
        Some((n, v)) => (n, Some(v)),
        None => (source, None),
    };
    let index = fetch_index(DEFAULT_REGISTRY, registry_ref)?;
    let entry = index.get(name).with_context(|| {
        format!("no plugin named {name:?} in {DEFAULT_REGISTRY}; try `vkdg plugin search {name}`")
    })?;
    if let Some(v) = wanted_version {
        if entry.version != v {
            anyhow::bail!(
                "{name} is at {} in the registry, not {v}; pin a version by installing its manifest URL directly",
                entry.version
            );
        }
    }
    let url = vkdg_plugin_host::registry_index::manifest_url(
        DEFAULT_REGISTRY,
        registry_ref,
        &entry.manifest_path,
    );
    let yaml = String::from_utf8(fetch_bytes(&url)?).context("manifest is not valid UTF-8")?;
    let manifest = RegistryManifest::from_yaml(&yaml).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok((manifest, None))
}

/// Fetch and validate a registry index.
fn fetch_index(registry: &str, git_ref: &str) -> Result<RegistryIndex> {
    let url = vkdg_plugin_host::registry_index::index_url(registry, git_ref);
    let json = String::from_utf8(fetch_bytes(&url)?).context("index is not valid UTF-8")?;
    RegistryIndex::from_json(&json).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Search the registry index.
fn plugin_search(query: &str, kind: Option<&PluginKind>) -> Result<()> {
    let index = fetch_index(DEFAULT_REGISTRY, "main")?;
    let hits = index.search(query, kind.map(manifest_kind_of));
    if hits.is_empty() {
        let shown = if query.is_empty() { "(all)" } else { query };
        println!("No plugins matching {shown} in {DEFAULT_REGISTRY}.");
        return Ok(());
    }
    println!("{} plugin(s) in {DEFAULT_REGISTRY}:", hits.len());
    for e in hits {
        println!("  {:<24} {:<10} {}", e.name, e.version, e.description);
        if !e.tags.is_empty() {
            println!("  {:<24} tags: {}", "", e.tags.join(", "));
        }
    }
    println!();
    println!("  Install with:  vkdg plugin install <name>");
    Ok(())
}

/// Map the CLI's kind flag onto the manifest schema's kind.
fn manifest_kind_of(kind: &PluginKind) -> vkdg_plugin_host::PluginManifestKind {
    use vkdg_plugin_host::PluginManifestKind as K;
    match kind {
        PluginKind::Provider => K::Provider,
        PluginKind::OauthProvider => K::OauthProvider,
        PluginKind::FilterPack => K::FilterPack,
        PluginKind::Compressor => K::Compressor,
        PluginKind::Router => K::Router,
    }
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    let resp = reqwest::blocking::get(url).with_context(|| format!("fetching {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("fetching {url} returned HTTP {status}");
    }
    Ok(resp.bytes()?.to_vec())
}

/// The config `vkdg setup` writes. Every line is indented explicitly: string
/// continuations (`\n\`) strip leading spaces, which once flattened `auth:`
/// and `models:` to column 0 and produced a file the gateway refused to load.
/// A custom endpoint becomes `openai-compat` with its `base_url`.
fn setup_yaml(
    provider_id: &str,
    base_url: Option<&str>,
    env_var: &str,
    model_pattern: &str,
) -> String {
    let models = model_pattern
        .split(',')
        .map(|p| format!("\"{}\"", p.trim()))
        .collect::<Vec<_>>()
        .join(", ");
    let conn = format!("{provider_id}-default");
    let mut lines = vec![
        "# VKDG Configuration, generated by 'vkdg setup'".to_owned(),
        "listen: \"0.0.0.0:8080\"".to_owned(),
        "connections:".to_owned(),
        format!("  - id: {conn}"),
        format!("    provider: {provider_id}"),
    ];
    if let Some(url) = base_url {
        lines.push(format!("    base_url: {url}"));
    }
    lines.extend([
        "    auth:".to_owned(),
        "      type: api_key".to_owned(),
        format!("      env_var: {env_var}"),
        format!("    models: [{models}]"),
        "    max_concurrent: 100".to_owned(),
        "    weight: 1".to_owned(),
        "routes:".to_owned(),
        "  - id: default".to_owned(),
        format!("    match_models: [{models}]"),
        "    strategy: round_robin".to_owned(),
        format!("    targets: [{conn}]"),
        "limits:".to_owned(),
        "  max_concurrent_requests: 1000".to_owned(),
    ]);
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

async fn test_connection_smoke(
    pipeline: &Arc<vkdg_http::PipelineState>,
    conn_id: &str,
) -> vkdg_admin::router::ConnectionTestResult {
    use std::time::Instant;
    use vkdg_core::{ApiType, ClientId, ConnectionId, RequestEnvelope, RequestId, TenantId};
    use vkdg_http::pipeline::run_conversation_pipeline;
    use vkdg_operations::{
        CapabilitySet, ConversationRequest, Message, MessageContent, Operation, Role,
    };

    // Find the connection's first concrete model so we don't send a glob.
    let model = {
        let Some(guard) = pipeline.catalog.get(&ConnectionId(conn_id.to_owned())) else {
            return vkdg_admin::router::ConnectionTestResult {
                latency_ms: 0,
                ok: false,
                error: Some(format!("connection '{conn_id}' not found")),
            };
        };
        let config = guard.read().await;
        config
            .config
            .models
            .iter()
            .find(|m| !m.contains(['*', '?']))
            .cloned()
            .unwrap_or_else(|| "test".to_owned())
    };

    // A synthetic envelope that routes directly to this connection by name.
    let envelope = RequestEnvelope {
        request_id: RequestId::new(),
        client_id: ClientId("_test".into()),
        tenant_id: TenantId("_test".into()),
        session_key: None,
        api_type: ApiType::AnthropicMessages,
        model_requested: model.clone(),
        deadline: None,
        mode_pack_override: None,
        compression_override: None,
        cache_bypass: true,
        include_think_tags: false,
        client_ip: None,
    };

    let op = Operation::Conversation(ConversationRequest {
        model,
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text("Hello".into()),
        }],
        tools: vec![],
        max_tokens: Some(1),
        temperature: None,
        stream: false,
        system: None,
        required_capabilities: CapabilitySet::default(),
        thinking: None,
    });

    // Force routing to this specific connection by temporarily making the
    // router irrelevant: we push the connection directly.
    use vkdg_core::pipeline::PipelineCtx;
    let mut ctx = PipelineCtx::new(envelope);
    ctx.connection_id = Some(ConnectionId(conn_id.to_owned()));
    ctx.route_id = Some("_test".to_owned());

    let t = Instant::now();
    let resp = run_conversation_pipeline(Arc::clone(pipeline), ctx, op).await;
    let latency_ms = t.elapsed().as_millis() as u64;

    let status = resp.status();
    if status.is_success() || status.as_u16() == 400 || status.as_u16() == 422 {
        // 400/422 = provider rejected the request (wrong model, etc.) — connection is alive.
        vkdg_admin::router::ConnectionTestResult {
            latency_ms,
            ok: true,
            error: None,
        }
    } else {
        let body = axum::body::to_bytes(resp.into_body(), 4096)
            .await
            .ok()
            .and_then(|b| String::from_utf8(b.to_vec()).ok())
            .unwrap_or_default();
        vkdg_admin::router::ConnectionTestResult {
            latency_ms,
            ok: false,
            error: Some(format!("HTTP {}: {}", status.as_u16(), body.trim())),
        }
    }
}

/// Sends one smoke request through a named connection and returns latency + status.
/// A provider-level error (wrong model, rate limit) still counts as ok=true because
/// the connection is alive. A network / auth failure returns ok=false.
#[cfg(test)]
mod tests {
    use super::*;

    fn loads(yaml: &str) -> Result<ConfigSnapshot, String> {
        let dir = std::env::temp_dir().join(format!("vkdg-setup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.yaml");
        std::fs::write(&path, yaml).unwrap();
        let r = load_and_validate(path.to_str().unwrap(), 0).map_err(|e| e.to_string());
        let _ = std::fs::remove_dir_all(dir);
        r
    }

    // `vkdg setup` wrote auth/models at column 0; the file never loaded.
    #[test]
    fn setup_config_loads_for_a_plugin_and_a_custom_endpoint() {
        let kiro = loads(&setup_yaml("kiro", None, "KIRO_API_KEY", "claude-*,auto")).unwrap();
        assert_eq!(kiro.connections[0].provider.adapter_id(), "kiro");
        assert_eq!(kiro.connections[0].models, ["claude-*", "auto"]);
        assert_eq!(kiro.routes.len(), 1);

        let custom = loads(&setup_yaml(
            "openai-compat",
            Some("http://localhost:11434"),
            "K",
            "*",
        ))
        .unwrap();
        let p = &custom.connections[0].provider;
        assert_eq!(
            (p.adapter_id(), p.as_str()),
            ("openai", "http://localhost:11434")
        );
        unknown_providers(&custom, &build_provider_registry()).unwrap();
    }

    #[test]
    fn unknown_providers_names_the_bad_connection_and_known_ids() {
        let snap = loads(&setup_yaml("openai-compatt", None, "K", "*")).unwrap();
        let err = unknown_providers(&snap, &build_provider_registry()).unwrap_err();
        assert!(err.contains("openai-compatt-default"), "{err}");
        assert!(err.contains("kiro") && err.contains("codex"), "{err}");
        let ok = loads(&setup_yaml("kiro", None, "K", "claude-*")).unwrap();
        unknown_providers(&ok, &build_provider_registry()).unwrap();
        unknown_providers(&ok, &ProviderRegistry::empty()).unwrap_err();
    }

    /// A component exporting `authenticate` that always returns `payload`.
    fn auth_component(payload: &str) -> Vec<u8> {
        let escaped = payload.replace('"', "\\22");
        let len = payload.len();
        wat::parse_str(format!(
            r#"(component
  (core module $m
    (memory (export "mem") 1)
    (data (i32.const 1024) "{escaped}")
    (func (export "f") (param i32 i32) (result i32)
      (i32.store (i32.const 0) (i32.const 1024))
      (i32.store (i32.const 4) (i32.const {len}))
      (i32.const 0))
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32) (i32.const 8192)))
  (core instance $i (instantiate $m))
  (func (export "authenticate") (param "input" string) (result string)
    (canon lift (core func $i "f") (memory (core memory $i "mem"))
      (realloc (core func $i "cabi_realloc")) string-encoding=utf8)))"#
        ))
        .unwrap()
    }

    // Installing an auth plugin used to need a restart. This drives the same
    // reload the admin API runs, against a real compiled component, and checks
    // the next authorization through the live registry the pipeline reads.
    #[tokio::test]
    async fn installed_auth_plugin_applies_after_reload_and_removal_denies() {
        let dir = tempfile::tempdir().unwrap();
        let store = PluginStore::new(dir.path());
        let providers = ProviderRegistry::empty();
        let hooks = Arc::new(vkdg_http::hooks::HookRegistry::default());
        let ids = ["gate".to_owned()];
        let req = || vkdg_http::hooks::HookRequest {
            key_id: "k".into(),
            tenant_id: "t".into(),
            client_ip: None,
            model: "m".into(),
            route_id: "guarded".into(),
        };
        assert!(
            hooks.authorize(&ids, req()).await.is_err(),
            "not installed: deny"
        );

        let plugin = dir.path().join("gate");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(
            plugin.join("manifest.yaml"),
            "name: gate\nversion: \"1.0.0\"\nkind: auth\ndescription: test\nlicense: MIT\n\
             install:\n  wasm: \"https://example.com/gate.wasm\"\n  checksum: \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n",
        )
        .unwrap();
        let allow = r#"{"result":"allowed","context":{"tenant_id":"t","key_id":"k","role":"user","scopes":[]}}"#;
        std::fs::write(plugin.join("plugin.wasm"), auth_component(allow)).unwrap();
        reload_plugins(&store, &providers, &hooks);
        assert!(
            hooks.authorize(&ids, req()).await.is_ok(),
            "installed: allow, no restart"
        );

        std::fs::remove_dir_all(&plugin).unwrap();
        reload_plugins(&store, &providers, &hooks);
        assert!(
            hooks.authorize(&ids, req()).await.is_err(),
            "removed: deny again"
        );
    }
}

//! VKDG gateway binary entry point.

use std::sync::Arc;

use anyhow::Result;
use axum::routing::{get, post};
use axum::Router;
use clap::{Parser, Subcommand};
use vkdg_config::{load_and_validate, ConfigSnapshot};
use vkdg_connections::{
    AuthKind, ConnectionCatalog, ConnectionConfig, CredentialManager, ProviderKind,
};
use vkdg_core::ConnectionId;
use vkdg_http::upstream::HttpClient;
use vkdg_http::{AdmissionGuard, AppState, PipelineState, ServerConfig};
use vkdg_observe::{init_tracing, DecisionRecordExporter, ObserveConfig};
use vkdg_operations::CapabilitySet;
use vkdg_provider_anthropic::AnthropicAdapter;
use vkdg_provider_antigravity::AntigravityAdapter;
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
use vkdg_provider_openai::OpenAIAdapter;
use vkdg_provider_sdk::ProviderRegistry;
use vkdg_provider_together::provider as together_provider;
use vkdg_routing::{PluginHooks, RouteConfig, RouteId, Router as VkdgRouter, StrategyKind};

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
        /// Plugin name, name@version, or URL to .wasm file.
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
        } => serve(config, listen, token).await?,
        Command::Doctor => vkdg_cli::commands::doctor::run().await?,
        Command::Config {
            sub: ConfigSub::Check { path },
        } => {
            vkdg_cli::commands::config_check::run(&path).await?;
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
    }
    Ok(())
}

// ── Serve ─────────────────────────────────────────────────────────────────────

async fn serve(
    config_path: Option<String>,
    listen: String,
    token_override: Option<String>,
) -> Result<()> {
    let _ = init_tracing(&ObserveConfig {
        otlp_endpoint: None,
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

    let pipeline = if let Some(path) = &config_path {
        match load_and_validate(path, 1) {
            Ok(snap) => {
                tracing::info!(path = %path, version = snap.version, "loaded config from file");
                let pipeline = build_pipeline_from_snapshot(&snap, max_concurrent);
                // Install hot-reload watcher; errors on bad reloads are logged, not fatal.
                let (tx, _rx) = vkdg_config::config_channel(snap);
                drop(vkdg_config::watch(path.clone(), tx, 1));
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

    // Extract catalog for admin API before pipeline is moved into AppState.
    let admin_catalog = pipeline.as_ref().map(|p| Arc::clone(&p.catalog));

    let mut state = AppState::new(server_config.clone());
    if let Some(p) = pipeline {
        state = state.with_pipeline(Arc::new(p));
    }

    // Build router here (not via vkdg_http::build_router) so we can add all
    // routes before calling .with_state() once. This avoids the Router<S> type
    // mismatch that comes from calling .route() on an already-resolved Router<()>.
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
    let dummy_snap = vkdg_config::ConfigSnapshot::default_empty();
    let (_config_tx, config_rx) = vkdg_config::config_channel(dummy_snap);
    let admin_state = vkdg_admin::AdminState {
        sessions: vkdg_admin::session::SessionStore::new(bootstrap_token),
        config_rx,
        started_at: std::sync::Arc::new(std::time::Instant::now()),
        key_store: vkdg_admin::session::KeyStore::new(),
        request_log: vkdg_admin::handlers::requests::RequestLog::new(),
        combo_resolver: None,
        catalog: admin_catalog,
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
        axum::serve(listener, admin_router).await.ok();
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

fn build_pipeline_from_snapshot(snap: &ConfigSnapshot, max_concurrent: usize) -> PipelineState {
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
        credentials: Arc::new(CredentialManager::new()),
        http_client: Arc::new(HttpClient::new()),
        exporter: Arc::new(DecisionRecordExporter::new()),
        provider_registry: build_provider_registry(),
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
        relay_enabled: false,
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
        credentials: Arc::new(CredentialManager::new()),
        http_client: Arc::new(HttpClient::new()),
        exporter: Arc::new(DecisionRecordExporter::new()),
        provider_registry: build_provider_registry(),
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
    })
}

fn build_provider_registry() -> Arc<ProviderRegistry> {
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
    Arc::new(r)
}

// ── Setup wizard ──────────────────────────────────────────────────────────────

fn cmd_setup() -> Result<()> {
    #[cfg(target_os = "linux")]
    use inquire::Confirm;
    use inquire::{Select, Text};

    println!("\n\x1b[1m\x1b[36m Welcome to VKDG Setup \x1b[0m\n");
    println!("This wizard will create a configuration file for your gateway.\n");

    // Config path
    let config_path = Text::new("Config file path:")
        .with_default("/etc/vkdg/config.yaml")
        .with_help_message("Where to save the VKDG configuration file")
        .prompt()?;

    // Listen address
    let listen = Text::new("Data API listen address:")
        .with_default("0.0.0.0:8080")
        .with_help_message("Port that AI clients will connect to")
        .prompt()?;

    // Provider selection
    let providers = vec![
        "anthropic",
        "openai",
        "groq (free tier available)",
        "gemini",
        "deepseek",
        "mistral",
        "custom (OpenAI-compatible endpoint)",
    ];
    let provider_choice = Select::new("First provider:", providers)
        .with_help_message("You can add more providers by editing the config file")
        .prompt()?;

    let provider_id = provider_choice.split(' ').next().unwrap_or("anthropic");

    let (provider_str, default_env, model_pattern) = match provider_id {
        "anthropic" => ("anthropic", "ANTHROPIC_API_KEY", "claude-*"),
        "openai" => ("openai", "OPENAI_API_KEY", "gpt-*"),
        "groq" => ("groq", "GROQ_API_KEY", "llama-*"),
        "gemini" => ("gemini", "GEMINI_API_KEY", "gemini-*"),
        "deepseek" => ("deepseek", "DEEPSEEK_API_KEY", "deepseek-*"),
        "mistral" => ("mistral", "MISTRAL_API_KEY", "mistral-*"),
        _ => ("openai-compat", "API_KEY", "*"),
    };

    let base_url_opt = if provider_id == "custom" {
        Some(
            Text::new("Base URL:")
                .with_placeholder("http://localhost:11434")
                .prompt()?,
        )
    } else {
        None
    };

    let env_var = Text::new("API key environment variable name:")
        .with_default(default_env)
        .prompt()?;

    let max_concurrent: u32 = Text::new("Max concurrent requests:")
        .with_default("100")
        .prompt()?
        .parse()
        .unwrap_or(100);

    // Build config YAML
    let base_url_line = base_url_opt
        .as_ref()
        .map(|u| format!("    base_url: {u}\n"))
        .unwrap_or_default();

    let config_yaml = format!(
        "# VKDG Configuration — generated by 'vkdg setup'\n\
         # Edit this file to add more connections and routes.\n\
         listen: \"{listen}\"\n\
         connections:\n\
         - id: {provider_str}-default\n\
         {base_url_line}  provider: {provider_str}\n\
           auth:\n\
             type: api_key\n\
             env_var: {env_var}\n\
           models: [\"{model_pattern}\"]\n\
           max_concurrent: {max_concurrent}\n\
           weight: 1\n\
         routes:\n\
         - id: default\n\
           match_models: [\"{model_pattern}\"]\n\
           strategy: round_robin\n\
           targets: [{provider_str}-default]\n\
         limits:\n\
           max_concurrent_requests: 1000\n"
    );

    // Create dir + write
    let path = std::path::Path::new(&config_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| eprintln!("Warning: could not create dir: {e}"));
    }

    match std::fs::write(path, &config_yaml) {
        Ok(()) => println!("\n\x1b[32m✓ Config written to {config_path}\x1b[0m"),
        Err(e) => {
            eprintln!("Could not write to {config_path}: {e}");
            eprintln!("Config:\n{config_yaml}");
        }
    }

    // Offer systemd install on Linux
    #[cfg(target_os = "linux")]
    if Confirm::new("Install as a systemd service?")
        .with_default(true)
        .prompt()
        .unwrap_or(false)
    {
        let user_mode = !is_root();
        cmd_install(&config_path, user_mode)?;
    }

    println!("\n\x1b[1mNext steps:\x1b[0m");
    println!("  export {env_var}=your-api-key-here");
    println!("  vkdg serve --config {config_path}");
    println!("  open http://localhost:9090  # admin console\n");

    Ok(())
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

fn handle_plugin(sub: PluginSub) {
    use PluginSub::*;
    match sub {
        Search { query, kind } => {
            let q = query.as_deref().unwrap_or("(all)");
            let kind_str = kind.map(|k| format!(" [kind={k:?}]")).unwrap_or_default();
            println!("Searching registry for: {q}{kind_str}");
            println!();
            println!("  Registry: https://github.com/vkdprojects/vkdg-registry");
            println!();
            println!("  The plugin registry client is coming in Phase 3.");
            println!(
                "  For now, browse: https://github.com/vkdprojects/vkdg-registry/tree/main/plugins"
            );
        }
        Install { plugin, .. } => {
            println!("Plugin install is coming in Phase 3.");
            println!();
            println!("  To use a plugin now:");
            println!("    1. Download the .wasm file");
            println!("    2. Drop it in ~/.config/vkdg/plugins/{plugin}/");
            println!("    3. Add 'plugins: [{plugin}]' to your vkdg.yaml");
        }
        List => {
            println!("Installed plugins: (none — plugin manager coming in Phase 3)");
            println!();
            println!("  Built-in providers (no install needed):");
            println!("    anthropic, openai, gemini, groq, deepseek, mistral, together, fireworks");
            println!("    claude-code, codex, kiro, kimi-coding, github-copilot, antigravity");
        }
        Remove { name } => {
            println!(
                "Plugin manager coming in Phase 3. Remove {name} manually from ~/.config/vkdg/plugins/"
            );
        }
        Update => {
            println!("Plugin manager coming in Phase 3.");
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
                eprintln!("Error reading {path}: {e}");
                std::process::exit(1);
            }
            Ok(content) => match serde_yaml::from_str::<serde_yaml::Value>(&content) {
                Ok(_) => println!("✓ {path}: valid YAML"),
                Err(e) => {
                    eprintln!("✗ {path}: invalid YAML: {e}");
                    std::process::exit(1);
                }
            },
        },
    }
}

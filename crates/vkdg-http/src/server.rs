use axum::{response::Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tracing::info;

use crate::frontdoor::ServerConfig;

pub async fn mcp_discovery() -> impl axum::response::IntoResponse {
    // Minimal MCP server discovery response.
    // Phase E: full MCP tool registry with provider, admin, and routing tools.
    Json(json!({
        "protocol_version": "2024-11-05",
        "server_info": {
            "name": "vkdg",
            "version": env!("CARGO_PKG_VERSION")
        },
        "capabilities": {
            "tools": {"listChanged": false}
        },
        "tools": [
            {
                "name": "route_preview",
                "description": "Preview which provider connection would handle a model request",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "model": {"type": "string", "description": "Model name or combo ID"}
                    },
                    "required": ["model"]
                }
            },
            {
                "name": "gateway_health",
                "description": "Check gateway status and active connections",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            }
        ]
    }))
}

pub async fn serve(config: ServerConfig, router: Router) -> anyhow::Result<()> {
    let addr = config.listen_addr.clone();
    let listener = TcpListener::bind(&addr).await?;
    info!(addr = %addr, "vkdg listening");

    // Connect info gives handlers the real socket peer, which is what client
    // IP checks must trust instead of a client-supplied header.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

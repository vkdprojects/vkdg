//! `vkdg healthcheck` — exit 0 when the gateway answers 2xx on `/health`.
//!
//! Exists so a `FROM scratch` image (no shell, no curl) can declare a Docker
//! HEALTHCHECK by running its own binary.

use anyhow::{bail, Context, Result};
use std::time::Duration;

/// Docker's healthcheck timeout is 3s in deploy/Dockerfile; stay under it so a
/// hung gateway reports "unhealthy" instead of "timed out".
const TIMEOUT: Duration = Duration::from_secs(2);

pub async fn run(url: &str) -> Result<()> {
    let res = tokio::time::timeout(TIMEOUT, reqwest::get(url))
        .await
        .with_context(|| format!("{url}: no response within {TIMEOUT:?}"))?
        .with_context(|| format!("{url}: unreachable"))?;
    if !res.status().is_success() {
        bail!("{url}: HTTP {}", res.status());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::run;
    use axum::{http::StatusCode, routing::get, Router};
    use std::time::{Duration, Instant};

    async fn serve(status: StatusCode) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route("/health", get(move || async move { status }));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/health")
    }

    // Refutes: a probe that reports failure even when the gateway is fine.
    #[tokio::test]
    async fn healthy_gateway_passes() {
        let url = serve(StatusCode::OK).await;
        assert!(run(&url).await.is_ok());
    }

    // Refutes: treating "got any HTTP response" as healthy. A gateway that is up
    // but answers 503 must fail the check, or Docker never restarts it.
    #[tokio::test]
    async fn unhealthy_status_fails() {
        let url = serve(StatusCode::SERVICE_UNAVAILABLE).await;
        let err = run(&url).await.unwrap_err();
        assert!(
            err.to_string().contains("503"),
            "error should name the status: {err}"
        );
    }

    // Refutes: a probe that hangs or passes when nothing listens. Docker kills a
    // healthcheck at its own timeout; ours must fail well before that.
    #[tokio::test]
    async fn unreachable_gateway_fails_fast() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/health", listener.local_addr().unwrap());
        drop(listener);
        let start = Instant::now();
        assert!(run(&url).await.is_err());
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "took {:?}",
            start.elapsed()
        );
    }
}

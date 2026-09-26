//! `vkdg request explain <id>` — fetch and display a DecisionRecord for a request.
//!
//! Queries the admin API at /admin/v1/requests/{id}.
//! Requires VKDG_ADMIN_URL (default: http://127.0.0.1:9090) and VKDG_ADMIN_SESSION cookie.
//!
//! Output formats:
//! - default: human-readable summary
//! - --json: machine-readable JSON

use anyhow::Result;

pub async fn run(request_id: &str, json_output: bool) -> Result<()> {
    let base = std::env::var("VKDG_ADMIN_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:9090".into());
    let session = std::env::var("VKDG_ADMIN_SESSION").unwrap_or_default();

    let url = format!("{}/admin/v1/requests/{}", base, request_id);

    let client = reqwest::Client::new();
    let mut req = client.get(&url);
    if !session.is_empty() {
        req = req.header("Cookie", format!("vkdg_session={}", session));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("failed to reach admin API at {}: {}", url, e))?;

    if resp.status() == 404 {
        anyhow::bail!("request {} not found in the recent request log", request_id);
    }
    if resp.status() == 401 {
        anyhow::bail!("unauthorized — set VKDG_ADMIN_SESSION to a valid session token");
    }
    if !resp.status().is_success() {
        anyhow::bail!("admin API returned {}", resp.status());
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("failed to parse response: {}", e))?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }

    // Human-readable output
    let unknown = serde_json::Value::String("unknown".into());
    let rid = body.get("request_id").unwrap_or(&unknown);
    let model = body.get("model").unwrap_or(&unknown);
    let status = body.get("status").unwrap_or(&unknown);
    let connection_id = body
        .get("connection_id")
        .and_then(|v| v.as_str())
        .unwrap_or("none");
    let duration = body
        .get("duration_ms")
        .and_then(|v| v.as_u64())
        .map(|ms| format!("{}ms", ms))
        .unwrap_or_else(|| "unknown".into());

    println!("Request: {}", rid);
    println!("  Model:      {}", model);
    println!("  Status:     {}", status);
    println!("  Connection: {}", connection_id);
    println!("  Duration:   {}", duration);

    if let Some(decision) = body.get("decision") {
        println!("  Decision:");
        if let Some(route) = decision.get("route_id") {
            println!("    Route:    {}", route);
        }
        if let Some(attempts) = decision.get("attempt_count") {
            println!("    Attempts: {}", attempts);
        }
        if let Some(excluded) = decision
            .get("candidates_excluded")
            .and_then(|v| v.as_array())
        {
            if !excluded.is_empty() {
                println!("    Excluded connections:");
                for e in excluded {
                    let id = e.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let reason = e.get("reason").and_then(|v| v.as_str()).unwrap_or("?");
                    println!("      {} ({})", id, reason);
                }
            }
        }
    }

    Ok(())
}

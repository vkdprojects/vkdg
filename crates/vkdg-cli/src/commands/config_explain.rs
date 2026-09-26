//! `vkdg config explain --model <model> [--tenant <tenant>]`
//!
//! Simulates routing for a model using the gateway's live eligibility logic.
//! Calls /admin/v1/routes/preview?model=<model>.
//! No quota consumed, no connection reserved.
//!
//! Output shows:
//! - Which combo (if any) the model resolves to
//! - Eligible connections with their signals
//! - Excluded connections with reasons

use anyhow::Result;

pub async fn run(model: &str, json_output: bool) -> Result<()> {
    let base = std::env::var("VKDG_ADMIN_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:9090".into());
    let session = std::env::var("VKDG_ADMIN_SESSION").unwrap_or_default();

    let encoded_model = urlencoding::encode(model);
    let url = format!("{}/admin/v1/routes/preview?model={}", base, encoded_model);

    let client = reqwest::Client::new();
    let mut req = client.get(&url);
    if !session.is_empty() {
        req = req.header("Cookie", format!("vkdg_session={}", session));
    }

    let resp = req.send().await
        .map_err(|e| anyhow::anyhow!("failed to reach admin API: {}", e))?;

    if resp.status() == 401 {
        anyhow::bail!("unauthorized -- set VKDG_ADMIN_SESSION");
    }
    if !resp.status().is_success() {
        anyhow::bail!("admin API returned {}", resp.status());
    }

    let body: serde_json::Value = resp.json().await
        .map_err(|e| anyhow::anyhow!("failed to parse response: {}", e))?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }

    let unknown = serde_json::Value::String("unknown".into());
    let model_out = body.get("model").unwrap_or(&unknown);
    println!("Model: {}", model_out);

    if let Some(combo) = body.get("combo_id").and_then(|v| v.as_str()) {
        println!("Resolved combo: {}", combo);
    }

    let eligible = body.get("eligible_connections")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_object().and_then(|o| o.get("id")).and_then(|i| i.as_str()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if eligible.is_empty() {
        println!("Eligible connections: none (request would fail with NoEligibleConnection)");
    } else {
        println!("Eligible connections ({}):", eligible.len());
        for c in &eligible {
            println!("  {}", c);
        }
    }

    let excluded = body.get("excluded_connections")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if !excluded.is_empty() {
        println!("Excluded connections ({}):", excluded.len());
        for e in &excluded {
            let id = e.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let reason = e.get("reason").and_then(|v| v.as_str()).unwrap_or("?");
            println!("  {} ({})", id, reason);
        }
    }

    Ok(())
}

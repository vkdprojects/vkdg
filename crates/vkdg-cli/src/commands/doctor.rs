//! `vkdg doctor` — environment and gateway health checks.

use anyhow::Result;

pub async fn run() -> Result<()> {
    println!("vkdg 0.1.0-rc1");
    println!();

    let mut all_ok = true;

    // Check 1: Rust toolchain
    let rust_version = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "not found".into());
    println!("  rustc:          {}", rust_version);

    // Check 2: RUST_LOG
    let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "(not set)".to_string());
    println!("  RUST_LOG:       {}", rust_log);
    println!();

    // Check 3: Gateway data plane reachability
    let base = std::env::var("VKDG_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8080".into());
    let data_plane_ok = check_url(&format!("{}/health", base)).await;
    print_check("data plane /health", &base, data_plane_ok);
    if !data_plane_ok {
        all_ok = false;
    }

    // Check 4: Admin API reachability
    let admin_base = std::env::var("VKDG_ADMIN_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:9090".into());
    let admin_ok = check_url(&format!("{}/admin/v1/system", admin_base)).await;
    print_check("admin API /system", &admin_base, admin_ok);
    if !admin_ok {
        all_ok = false;
    }

    // Check 5: ANTHROPIC_API_KEY present
    let anthropic_key = std::env::var("ANTHROPIC_API_KEY");
    print_check("ANTHROPIC_API_KEY", "env var", anthropic_key.is_ok());
    if anthropic_key.is_err() {
        all_ok = false;
    }

    // Check 6: VKDG_ADMIN_URL configured (always informational)
    print_check("VKDG_ADMIN_URL", &admin_base, true);

    println!();
    if all_ok {
        println!("All checks passed.");
    } else {
        println!("Some checks failed. Start the gateway: cargo run -p vkdg -- serve");
    }

    Ok(())
}

async fn check_url(url: &str) -> bool {
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reqwest::get(url),
    )
    .await
    .ok()
    .and_then(|r| r.ok())
    .map(|r| r.status().is_success())
    .unwrap_or(false)
}

fn print_check(name: &str, detail: &str, ok: bool) {
    let status = if ok { "ok  " } else { "FAIL" };
    println!("  [{status}]  {name:<28} {detail}");
}

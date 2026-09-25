use anyhow::Result;

/// Print version and basic environment health.
pub async fn run() -> Result<()> {
    println!("vkdg 0.1.0");

    let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "(not set)".to_string());
    println!("RUST_LOG: {}", rust_log);

    println!("OK: environment looks fine");
    Ok(())
}

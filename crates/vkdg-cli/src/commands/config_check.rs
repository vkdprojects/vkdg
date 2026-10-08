use anyhow::Result;
use vkdg_config::load_and_validate;

/// Check that `path` points to a valid, fully-referenced gateway config.
pub async fn run(path: &str) -> Result<()> {
    match load_and_validate(path, 0) {
        Ok(snap) => {
            println!(
                "OK: config valid (version {}, {} connections, {} routes)",
                snap.version,
                snap.connections.len(),
                snap.routes.len()
            );
            Ok(())
        }
        Err(e) => anyhow::bail!("config invalid: {e}"),
    }
}

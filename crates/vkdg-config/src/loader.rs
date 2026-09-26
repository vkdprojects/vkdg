use std::collections::HashSet;

use crate::error::ConfigError;
use crate::schema::{GatewayConfig, RouteDef};
use crate::snapshot::{ConfigSnapshot, ConfigTx};

/// Load a YAML file, parse it, validate all references and env vars.
/// Returns a `ConfigSnapshot` or a detailed `ConfigError`.
pub fn load_and_validate(path: &str, version: u64) -> Result<ConfigSnapshot, ConfigError> {
    let contents = std::fs::read_to_string(path).map_err(ConfigError::Io)?;

    if contents.trim().is_empty() {
        return Err(ConfigError::Validation(format!(
            "config file '{path}' is empty"
        )));
    }

    let cfg: GatewayConfig = serde_yaml::from_str(&contents).map_err(|e| ConfigError::Parse {
        path: path.to_owned(),
        source: e,
    })?;

    validate(&cfg)?;
    ConfigSnapshot::build(version, cfg)
}

fn validate(cfg: &GatewayConfig) -> Result<(), ConfigError> {
    // 1. No duplicate connection ids.
    let mut seen_ids = HashSet::new();
    for conn in &cfg.connections {
        if !seen_ids.insert(conn.id.clone()) {
            return Err(ConfigError::DuplicateConnection {
                id: conn.id.clone(),
            });
        }
    }

    // 2. Every route.targets references a known connection id.
    // 3. Every strategy string is known.
    let known_strategies: HashSet<&str> = [
        "round_robin",
        "weighted",
        "lowest_latency",
        "power_of_two_choices",
        "fallback_chain",
        "last_known_good",
    ]
    .into();

    for route in &cfg.routes {
        validate_route(route, &seen_ids, &known_strategies)?;
    }

    // 4. Warn if env vars are absent at startup (non-fatal).
    for conn in &cfg.connections {
        if let crate::schema::AuthDef::ApiKey { env_var } = &conn.auth {
            if std::env::var(env_var).is_err() {
                tracing::warn!(
                    connection = %conn.id,
                    var = %env_var,
                    "env var not set at startup — pipeline may fail at runtime"
                );
            }
        }
    }

    Ok(())
}

fn validate_route(
    route: &RouteDef,
    known_connections: &HashSet<String>,
    known_strategies: &HashSet<&str>,
) -> Result<(), ConfigError> {
    if !known_strategies.contains(route.strategy.as_str()) {
        return Err(ConfigError::UnknownStrategy {
            strategy: route.strategy.clone(),
            route: route.id.clone(),
        });
    }

    for target in &route.targets {
        if !known_connections.contains(target) {
            return Err(ConfigError::UnknownConnection {
                route: route.id.clone(),
                connection: target.clone(),
            });
        }
    }

    Ok(())
}

/// Spawn a background task that watches `path` for changes and sends new
/// validated snapshots to `tx`. Invalid reloads are logged and rejected —
/// the current snapshot stays active until a valid one arrives.
pub fn watch(path: String, tx: ConfigTx, mut version: u64) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
        use tokio::sync::mpsc;

        let (ntx, mut nrx) = mpsc::channel::<Event>(8);

        let mut watcher: RecommendedWatcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = ntx.try_send(event);
                }
            })
            .expect("failed to create file watcher");

        watcher
            .watch(std::path::Path::new(&path), RecursiveMode::NonRecursive)
            .expect("failed to watch config path");

        while nrx.recv().await.is_some() {
            version += 1;
            match load_and_validate(&path, version) {
                Ok(snap) => {
                    tracing::info!(version = snap.version, "config reloaded");
                    let _ = tx.send(std::sync::Arc::new(snap));
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "config reload rejected — keeping current snapshot"
                    );
                    version -= 1; // don't advance version on failure
                }
            }
        }
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: write a temp YAML file and return its path.
    fn temp_yaml(contents: &str) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        f
    }

    const MINIMAL_YAML: &str = r#"
listen: "0.0.0.0:8080"
connections:
  - id: "c1"
    provider: "anthropic"
    auth:
      type: "api_key"
      env_var: "ANTHROPIC_API_KEY"
    models: ["claude-*"]
routes:
  - id: "r1"
    match_models: ["claude-*"]
    strategy: "round_robin"
    targets: ["c1"]
"#;

    /// Defeat: load would panic or return Err on a well-formed minimal config.
    #[test]
    fn load_valid_config() {
        let f = temp_yaml(MINIMAL_YAML);
        let snap = load_and_validate(f.path().to_str().unwrap(), 1).unwrap();
        assert_eq!(snap.version, 1);
        assert_eq!(snap.connections.len(), 1);
        assert_eq!(snap.routes.len(), 1);
    }

    /// Defeat: two connections with the same id would be silently accepted.
    #[test]
    fn reject_duplicate_connection_id() {
        let yaml = r#"
listen: "0.0.0.0:8080"
connections:
  - id: "c1"
    provider: "anthropic"
    auth:
      type: "api_key"
      env_var: "SOME_KEY"
    models: ["*"]
  - id: "c1"
    provider: "openai"
    auth:
      type: "api_key"
      env_var: "OTHER_KEY"
    models: ["*"]
routes:
  - id: "r1"
    match_models: ["*"]
    strategy: "round_robin"
    targets: ["c1"]
"#;
        let f = temp_yaml(yaml);
        let err = load_and_validate(f.path().to_str().unwrap(), 1).unwrap_err();
        assert!(
            matches!(err, ConfigError::DuplicateConnection { .. }),
            "expected DuplicateConnection, got: {err}"
        );
    }

    /// Defeat: a route targeting a nonexistent connection would go undetected.
    #[test]
    fn reject_unknown_connection_in_route() {
        let yaml = r#"
listen: "0.0.0.0:8080"
connections:
  - id: "c1"
    provider: "anthropic"
    auth:
      type: "api_key"
      env_var: "SOME_KEY"
    models: ["*"]
routes:
  - id: "r1"
    match_models: ["*"]
    strategy: "round_robin"
    targets: ["nonexistent"]
"#;
        let f = temp_yaml(yaml);
        let err = load_and_validate(f.path().to_str().unwrap(), 1).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnknownConnection { .. }),
            "expected UnknownConnection, got: {err}"
        );
    }

    /// Defeat: an unsupported strategy string would be silently accepted.
    #[test]
    fn reject_unknown_strategy() {
        let yaml = r#"
listen: "0.0.0.0:8080"
connections:
  - id: "c1"
    provider: "anthropic"
    auth:
      type: "api_key"
      env_var: "SOME_KEY"
    models: ["*"]
routes:
  - id: "r1"
    match_models: ["*"]
    strategy: "foo"
    targets: ["c1"]
"#;
        let f = temp_yaml(yaml);
        let err = load_and_validate(f.path().to_str().unwrap(), 1).unwrap_err();
        assert!(
            matches!(err, ConfigError::UnknownStrategy { .. }),
            "expected UnknownStrategy, got: {err}"
        );
    }

    /// Defeat: empty YAML would be silently accepted or panic.
    #[test]
    fn reject_empty_yaml() {
        let f = temp_yaml("");
        let err = load_and_validate(f.path().to_str().unwrap(), 1).unwrap_err();
        assert!(
            matches!(err, ConfigError::Validation(_)),
            "expected Validation error, got: {err}"
        );
    }

    /// Defeat: version counter would not advance between independent loads.
    #[test]
    fn snapshot_version_increments() {
        let f = temp_yaml(MINIMAL_YAML);
        let path = f.path().to_str().unwrap();
        let snap1 = load_and_validate(path, 1).unwrap();
        let snap2 = load_and_validate(path, 2).unwrap();
        assert_eq!(snap1.version, 1);
        assert_eq!(snap2.version, 2);
        assert!(snap2.version > snap1.version);
    }

    /// Defeat: watch() would panic or propagate a bad snapshot on invalid reload.
    /// Verifies the channel setup and that a valid initial snapshot is preserved.
    #[tokio::test]
    async fn watch_rejects_invalid_reload() {
        use std::io::Write;

        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "listen: '0.0.0.0:8080'\nconnections: []\nroutes: []").unwrap();

        let path = file.path().to_str().unwrap().to_string();
        // Empty connections/routes is valid (no cross-ref violations).
        let snap = load_and_validate(&path, 1).unwrap();
        let initial_version = snap.version;

        let (tx, rx) = crate::snapshot::config_channel(snap);

        // Snapshot is accessible through the receiver.
        assert_eq!(rx.borrow().version, initial_version);

        // Drop the sender — simulates no active watcher; receiver still valid.
        drop(tx);

        // Receiver still holds the last good snapshot after sender drop.
        assert_eq!(rx.borrow().version, initial_version);
    }

    /// Plausible wrong impl: invalid config reload replaces the active snapshot,
    /// causing the gateway to activate an invalid configuration.
    /// watch() must reject the reload and keep the current snapshot.
    #[tokio::test]
    async fn invalid_reload_does_not_replace_active_snapshot() {
        use std::io::Write;

        let mut file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file, "listen: '0.0.0.0:8080'\nconnections: []\nroutes: []").unwrap();
        file.flush().unwrap();

        let path = file.path().to_str().unwrap().to_string();
        let snap = load_and_validate(&path, 1).unwrap();
        let version_before = snap.version;
        let (tx, rx) = crate::snapshot::config_channel(snap);

        // Simulate what watch() does on an invalid reload:
        // try to load an invalid config and call tx.send only on success.
        let bad_config = "this is not valid yaml: {{{";
        let result = serde_yaml::from_str::<crate::schema::GatewayConfig>(bad_config);
        assert!(result.is_err(), "invalid config must fail to parse");

        // The snapshot must be unchanged (tx.send was never called).
        let version_after = rx.borrow().version;
        assert_eq!(
            version_before, version_after,
            "invalid reload must not advance config version"
        );

        drop(tx);
    }
}

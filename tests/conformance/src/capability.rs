// Contract: capability mismatch → explicit exclusion from eligible set,
//           NEVER fail-open (i.e., never silently route to an incapable connection).

use vkdg_connections::{AuthKind, ConnectionCatalog, ConnectionConfig, ProviderKind};
use vkdg_core::ConnectionId;
use vkdg_operations::{Capability, CapabilitySet};

// ── Test helpers ──────────────────────────────────────────────────────────────

fn make_config(id: &str, capabilities: CapabilitySet) -> ConnectionConfig {
    ConnectionConfig {
        id: ConnectionId(id.to_string()),
        provider: ProviderKind::Anthropic,
        auth: AuthKind::ApiKey {
            env_var: "DUMMY_KEY".into(),
        },
        models: vec!["claude-*".into()],
        max_concurrent: 10,
        weight: 1,
        tags: vec![],
        capabilities,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Wrong impl this catches: eligible_for_operation that ignores the capability
/// check (e.g. falls through to eligible() without filtering) would return the
/// connection even though its capabilities set is empty, causing silent
/// fail-open routing to an incapable backend.
#[test]
fn unsupported_capability_is_explicit_error_not_fail_open() {
    // Connection declares NO capabilities.
    let config = make_config("conn-no-caps", CapabilitySet::default());
    let catalog = ConnectionCatalog::new(vec![config]);

    // Caller requires Vision.
    let mut required = CapabilitySet::new();
    required.insert(Capability::Vision);

    let eligible = catalog.eligible_for_operation("claude-3-5-sonnet-20241022", &[], &required);

    assert!(
        eligible.is_empty(),
        "connection with empty capabilities must NOT be selected for a Vision request (fail-open \
         is a security/correctness violation); got: {eligible:?}"
    );
}

/// Wrong impl this catches: a capability check that uses == instead of
/// is_superset, causing a connection that supports exactly the required set to
/// be incorrectly excluded.
#[test]
fn connection_with_exact_capability_is_eligible() {
    let mut caps = CapabilitySet::new();
    caps.insert(Capability::Vision);
    let config = make_config("conn-vision", caps);
    let catalog = ConnectionCatalog::new(vec![config]);

    let mut required = CapabilitySet::new();
    required.insert(Capability::Vision);

    let eligible = catalog.eligible_for_operation("claude-3-5-sonnet-20241022", &[], &required);

    assert_eq!(
        eligible,
        vec![ConnectionId("conn-vision".into())],
        "connection that declares Vision must be returned when Vision is required"
    );
}

/// Wrong impl this catches: a capability check that requires empty required_caps
/// to return nothing — empty means "no filter", all healthy connections pass.
#[test]
fn empty_required_caps_matches_all_healthy_connections() {
    let config = make_config("conn-no-caps", CapabilitySet::default());
    let catalog = ConnectionCatalog::new(vec![config]);

    // Empty required set — no capability filter applied.
    let eligible = catalog.eligible_for_operation(
        "claude-3-5-sonnet-20241022",
        &[],
        &CapabilitySet::default(),
    );

    assert_eq!(
        eligible,
        vec![ConnectionId("conn-no-caps".into())],
        "empty required_caps must not filter out any healthy connection"
    );
}

/// Wrong impl this catches: a superset check that passes when only one of two
/// required capabilities is present (partial match accepted as valid).
#[test]
fn partial_capability_match_is_not_eligible() {
    let mut caps = CapabilitySet::new();
    caps.insert(Capability::Vision); // has Vision, but not Tools
    let config = make_config("conn-partial", caps);
    let catalog = ConnectionCatalog::new(vec![config]);

    let mut required = CapabilitySet::new();
    required.insert(Capability::Vision);
    required.insert(Capability::Tools);

    let eligible = catalog.eligible_for_operation("claude-3-5-sonnet-20241022", &[], &required);

    assert!(
        eligible.is_empty(),
        "connection missing Tools must not match a Vision+Tools requirement; got: {eligible:?}"
    );
}

/// Unit check: CapabilitySet operations work correctly.
/// PASSES — these are already-implemented standard collection operations.
#[test]
fn capability_set_contains_and_insert() {
    let mut caps = CapabilitySet::new();
    assert!(!caps.contains(&Capability::Vision));

    caps.insert(Capability::Vision);
    assert!(caps.contains(&Capability::Vision));
    assert!(!caps.contains(&Capability::Tools));
}

/// Documents all Capability variants exist (compile-time completeness check).
/// PASSES.
#[test]
fn all_capability_variants_exist() {
    let caps = vec![
        Capability::Tools,
        Capability::Reasoning,
        Capability::Vision,
        Capability::JsonSchema,
        Capability::Streaming,
        Capability::Embedding,
        Capability::ImageInput,
        Capability::AudioInput,
        Capability::VideoInput,
    ];
    assert_eq!(caps.len(), 9, "Spec defines 9 Capability variants");
}

// ── IP policy (scenario: ip_allowlist_blocks_request) ─────────────────────────

/// Plausible wrong impl: IP blocklist doesn't override allowlist — a blocked
/// IP that also matches the allowlist prefix is incorrectly allowed through.
/// Scenario: spec/scenarios/ip_allowlist_blocks_request.yaml
/// PASSES — blocklist wins is already implemented.
#[test]
fn ip_policy_blocklist_wins_over_allowlist() {
    use vkdg_http::IpPolicy;
    let p = IpPolicy {
        allowlist: vec!["192.168.1.".into()],
        blocklist: vec!["192.168.1.100".into()],
    };
    assert!(
        !p.allows("192.168.1.100"),
        "blocked IP must be rejected even if it matches the allowlist prefix"
    );
    assert!(
        p.allows("192.168.1.200"),
        "non-blocked IP in allowlist prefix must be allowed"
    );
}

/// Plausible wrong impl: non-empty allowlist with no matching entry silently
/// allows the request instead of blocking it.
/// Scenario: spec/scenarios/ip_allowlist_blocks_request.yaml (contract: unlisted IPs blocked)
/// PASSES.
#[test]
fn ip_policy_allowlist_blocks_unlisted_ip() {
    use vkdg_http::IpPolicy;
    let p = IpPolicy {
        allowlist: vec!["10.0.0.1".into()],
        blocklist: vec![],
    };
    assert!(
        !p.allows("192.168.1.1"),
        "IP not in allowlist must be blocked when allowlist is non-empty"
    );
    assert!(
        p.allows("10.0.0.1"),
        "IP exactly matching allowlist entry must be allowed"
    );
}

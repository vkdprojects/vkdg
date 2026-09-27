use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use vkdg_core::net::IpNet;

/// Opaque virtual key identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VirtualKeyId(pub String);

/// What a virtual key may call on the data plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyScope {
    /// Conversation endpoints: `/v1/messages`, `/v1/chat/completions`.
    DataInference,
    /// Image generation: `/v1/images/generations`.
    DataImage,
}

impl KeyScope {
    /// Scopes a new key gets when the caller does not choose.
    pub const DEFAULT: &'static [KeyScope] = &[KeyScope::DataInference, KeyScope::DataImage];
}

/// Prefix of every raw token, so leaked keys are recognizable by secret scanners.
pub const TOKEN_PREFIX: &str = "vkdg_";

/// Characters of the raw token kept for display (`vkdg_` plus 8 random chars).
const DISPLAY_PREFIX_LEN: usize = TOKEN_PREFIX.len() + 8;

/// A client API key for the data plane.
///
/// The raw token is shown once at creation and never stored: only its SHA-256
/// hash and a short display prefix are kept.
#[derive(Clone)]
pub struct VirtualKey {
    pub id: VirtualKeyId,
    pub name: String,
    pub tenant_id: String,
    /// SHA-256 hex of the raw token.
    pub token_hash: String,
    /// First characters of the raw token, safe to show (`vkdg_1a2b3c4d`).
    pub prefix: String,
    pub scopes: Vec<KeyScope>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    /// Set while the key is disabled. Unlike revocation this is reversible.
    pub disabled_at: Option<DateTime<Utc>>,
    /// After this instant the key no longer authenticates.
    pub expires_at: Option<DateTime<Utc>>,
    /// Model patterns (`claude-*`) this key may call. Empty = every model.
    pub allowed_models: Vec<String>,
    /// Client ranges this key may be used from. Empty = anywhere.
    pub allowed_ips: Vec<IpNet>,
    /// Tokens (input + output) this key may use per calendar month (UTC).
    pub monthly_token_limit: Option<u64>,
    /// Requests this key may start per minute.
    pub requests_per_minute: Option<u32>,
    /// Keep this key's requests out of the request history. Usage is still
    /// counted against its limits.
    pub no_log: bool,
}

/// Changes to an existing key's policy. `None` leaves a field as it is; for
/// the `Option` fields, `Some(None)` clears the limit.
#[derive(Debug, Clone, Default)]
pub struct KeyPatch {
    pub name: Option<String>,
    pub scopes: Option<Vec<KeyScope>>,
    pub expires_at: Option<Option<DateTime<Utc>>>,
    pub allowed_models: Option<Vec<String>>,
    pub allowed_ips: Option<Vec<IpNet>>,
    pub monthly_token_limit: Option<Option<u64>>,
    pub requests_per_minute: Option<Option<u32>>,
    pub no_log: Option<bool>,
}

/// What a new key may do. `Default` is a key with every data-plane scope and no
/// restriction beyond that.
#[derive(Debug, Clone)]
pub struct NewKey {
    pub name: String,
    pub tenant_id: String,
    pub scopes: Vec<KeyScope>,
    pub expires_at: Option<DateTime<Utc>>,
    pub allowed_models: Vec<String>,
    pub allowed_ips: Vec<IpNet>,
    pub monthly_token_limit: Option<u64>,
    pub requests_per_minute: Option<u32>,
    pub no_log: bool,
}

impl NewKey {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            tenant_id: "default".into(),
            scopes: KeyScope::DEFAULT.to_vec(),
            expires_at: None,
            allowed_models: vec![],
            allowed_ips: vec![],
            monthly_token_limit: None,
            requests_per_minute: None,
            no_log: false,
        }
    }
}

impl std::fmt::Debug for VirtualKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The hash is not the secret, but there is no reason to print it either.
        f.debug_struct("VirtualKey")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("prefix", &self.prefix)
            .field("scopes", &self.scopes)
            .field("revoked", &self.revoked_at.is_some())
            .finish_non_exhaustive()
    }
}

impl VirtualKey {
    /// Mint a new key. Returns `(VirtualKey, raw_token)`; the raw token is shown once.
    pub fn new(spec: NewKey) -> (Self, String) {
        let raw = mint_token();
        let key = Self {
            id: VirtualKeyId(Uuid::new_v4().to_string()),
            name: spec.name,
            tenant_id: spec.tenant_id,
            token_hash: hash_token(&raw),
            prefix: display_prefix(&raw),
            scopes: spec.scopes,
            created_at: Utc::now(),
            last_used_at: None,
            revoked_at: None,
            disabled_at: None,
            expires_at: spec.expires_at,
            allowed_models: spec.allowed_models,
            allowed_ips: spec.allowed_ips,
            monthly_token_limit: spec.monthly_token_limit,
            requests_per_minute: spec.requests_per_minute,
            no_log: spec.no_log,
        };
        (key, raw)
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled_at.is_some()
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|t| now >= t)
    }

    /// Whether this key may call `model`.
    pub fn permits_model(&self, model: &str) -> bool {
        self.allowed_models.is_empty() || vkdg_core::glob::matches_any(&self.allowed_models, model)
    }

    /// Whether this key may be used from `ip` (unknown fails a non-empty list).
    pub fn permits_ip(&self, ip: Option<std::net::IpAddr>) -> bool {
        if self.allowed_ips.is_empty() {
            return true;
        }
        ip.is_some_and(|ip| self.allowed_ips.iter().any(|n| n.contains(ip)))
    }

    pub fn allows(&self, scope: KeyScope) -> bool {
        self.scopes.contains(&scope)
    }
}

/// A fresh raw token. 128 bits of randomness behind a recognisable prefix.
pub(crate) fn mint_token() -> String {
    format!("{TOKEN_PREFIX}{}", Uuid::new_v4().simple())
}

/// The safe-to-show start of a raw token.
pub(crate) fn display_prefix(raw: &str) -> String {
    raw[..DISPLAY_PREFIX_LEN.min(raw.len())].to_owned()
}

/// SHA-256 hex of a raw token: the only form a token is stored or looked up in.
pub fn hash_token(raw: &str) -> String {
    format!("{:x}", Sha256::digest(raw.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Plausible wrong impl: the raw token is kept on the record and ends up in
    // the database or a log line.
    #[test]
    fn new_key_keeps_only_hash_and_display_prefix() {
        let (key, raw) = VirtualKey::new(NewKey::named("ci"));
        assert!(raw.starts_with(TOKEN_PREFIX));
        assert_eq!(key.token_hash, hash_token(&raw));
        assert!(raw.starts_with(&key.prefix) && key.prefix.len() < raw.len());
        let dbg = format!("{key:?}");
        assert!(!dbg.contains(&raw[DISPLAY_PREFIX_LEN..]), "{dbg}");
        assert!(!dbg.contains(&key.token_hash), "{dbg}");
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    fn key(f: impl FnOnce(&mut NewKey)) -> VirtualKey {
        let mut spec = NewKey::named("k");
        f(&mut spec);
        VirtualKey::new(spec).0
    }

    #[test]
    fn expiry_is_exclusive_of_the_past_only() {
        let now = Utc::now();
        assert!(!key(|_| {}).is_expired_at(now), "no expiry = never");
        let k = key(|s| s.expires_at = Some(now));
        assert!(k.is_expired_at(now), "expires_at itself is already expired");
        assert!(!k.is_expired_at(now - chrono::Duration::seconds(1)));
    }

    #[test]
    fn model_list_uses_the_shared_pattern_syntax() {
        assert!(key(|_| {}).permits_model("anything"), "empty = every model");
        let k = key(|s| s.allowed_models = vec!["claude-*".into(), "gpt-4o-mini".into()]);
        assert!(k.permits_model("claude-sonnet-4.5"));
        assert!(k.permits_model("gpt-4o-mini"));
        assert!(!k.permits_model("gpt-4o"));
    }

    #[test]
    fn ip_list_fails_closed_on_unknown_address() {
        let any = key(|_| {});
        assert!(any.permits_ip(None), "no list = anywhere");
        let k = key(|s| s.allowed_ips = vec!["10.0.0.0/8".parse().unwrap()]);
        assert!(k.permits_ip(Some("10.2.3.4".parse().unwrap())));
        assert!(!k.permits_ip(Some("192.168.0.1".parse().unwrap())));
        assert!(
            !k.permits_ip(None),
            "unknown client cannot prove membership"
        );
    }
}

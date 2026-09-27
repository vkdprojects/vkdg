use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

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
#[derive(Clone, Serialize, Deserialize)]
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
    pub fn new(name: String, tenant_id: String, scopes: Vec<KeyScope>) -> (Self, String) {
        let raw = format!("{TOKEN_PREFIX}{}", Uuid::new_v4().simple());
        let key = Self {
            id: VirtualKeyId(Uuid::new_v4().to_string()),
            name,
            tenant_id,
            token_hash: hash_token(&raw),
            prefix: raw[..DISPLAY_PREFIX_LEN].to_owned(),
            scopes,
            created_at: Utc::now(),
            last_used_at: None,
            revoked_at: None,
        };
        (key, raw)
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }

    pub fn allows(&self, scope: KeyScope) -> bool {
        self.scopes.contains(&scope)
    }
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
        let (key, raw) = VirtualKey::new("ci".into(), "t".into(), KeyScope::DEFAULT.to_vec());
        assert!(raw.starts_with(TOKEN_PREFIX));
        assert_eq!(key.token_hash, hash_token(&raw));
        assert!(raw.starts_with(&key.prefix) && key.prefix.len() < raw.len());
        let dbg = format!("{key:?}");
        assert!(!dbg.contains(&raw[DISPLAY_PREFIX_LEN..]), "{dbg}");
        assert!(!dbg.contains(&key.token_hash), "{dbg}");
    }
}

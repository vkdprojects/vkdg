//! Artifact handles with tenant ACL and TTL-based expiry.
//!
//! Phase D uses an in-memory store; Phase E can swap in object storage
//! by implementing the same interface.

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

// ── ArtifactHandle ────────────────────────────────────────────────────────────

/// Opaque handle for an artifact. Never exposes internal paths to callers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ArtifactHandle(pub Uuid);

impl ArtifactHandle {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ArtifactHandle {
    fn default() -> Self {
        Self::new()
    }
}

// ── ArtifactMeta ──────────────────────────────────────────────────────────────

/// Metadata for a stored artifact. Binary content is held separately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactMeta {
    pub handle: ArtifactHandle,
    /// Tenant that owns this artifact.
    pub owner_tenant: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

// ── ArtifactError ─────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("artifact not found: {0}")]
    NotFound(Uuid),

    #[error("artifact expired: {0}")]
    Expired(Uuid),

    #[error("access denied: artifact belongs to different tenant")]
    Forbidden,

    #[error("size exceeds limit: {size} > {limit}")]
    TooLarge { size: u64, limit: u64 },

    #[error("mime type not allowed: {0}")]
    MimeNotAllowed(String),
}

// ── MIME allow-list ───────────────────────────────────────────────────────────

/// Returns `true` if `mime` is in the approved allow-list.
pub fn allowed_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/png"
            | "image/jpeg"
            | "image/webp"
            | "image/gif"
            | "video/mp4"
            | "audio/mpeg"
            | "audio/wav"
            | "application/octet-stream"
    )
}

// ── InMemoryArtifactStore ─────────────────────────────────────────────────────

type Inner = Arc<RwLock<HashMap<ArtifactHandle, (ArtifactMeta, Bytes)>>>;

/// In-memory artifact store for Phase D.
///
/// Thread-safe via `Arc<RwLock<_>>`. Clone to share across tasks.
#[derive(Clone)]
pub struct InMemoryArtifactStore {
    inner: Inner,
    size_limit_bytes: u64,
}

impl InMemoryArtifactStore {
    pub fn new(size_limit_bytes: u64) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            size_limit_bytes,
        }
    }

    /// Store an artifact and return its handle.
    ///
    /// Rejects payloads that exceed `size_limit_bytes` or use a disallowed MIME.
    pub async fn store(
        &self,
        owner_tenant: String,
        mime_type: String,
        data: Bytes,
        ttl_secs: u64,
    ) -> Result<ArtifactHandle, ArtifactError> {
        if data.len() as u64 > self.size_limit_bytes {
            return Err(ArtifactError::TooLarge {
                size: data.len() as u64,
                limit: self.size_limit_bytes,
            });
        }
        if !allowed_mime(&mime_type) {
            return Err(ArtifactError::MimeNotAllowed(mime_type));
        }
        let handle = ArtifactHandle::new();
        let meta = ArtifactMeta {
            handle: handle.clone(),
            owner_tenant,
            mime_type,
            size_bytes: data.len() as u64,
            expires_at: Utc::now() + Duration::seconds(ttl_secs as i64),
            created_at: Utc::now(),
        };
        self.inner
            .write()
            .await
            .insert(handle.clone(), (meta, data));
        Ok(handle)
    }

    /// Retrieve an artifact by handle, enforcing tenant ownership and TTL.
    pub async fn get(
        &self,
        handle: &ArtifactHandle,
        requesting_tenant: &str,
    ) -> Result<(ArtifactMeta, Bytes), ArtifactError> {
        let map = self.inner.read().await;
        let (meta, data) = map.get(handle).ok_or(ArtifactError::NotFound(handle.0))?;
        if meta.owner_tenant != requesting_tenant {
            return Err(ArtifactError::Forbidden);
        }
        if Utc::now() > meta.expires_at {
            return Err(ArtifactError::Expired(handle.0));
        }
        Ok((meta.clone(), data.clone()))
    }

    /// Remove all expired artifacts. Returns the count of removed entries.
    pub async fn purge_expired(&self) -> usize {
        let mut map = self.inner.write().await;
        let before = map.len();
        let now = Utc::now();
        map.retain(|_, (meta, _)| meta.expires_at > now);
        before - map.len()
    }

    /// Test-only: insert an artifact with an explicit `expires_at` timestamp,
    /// bypassing TTL calculation.  Allows expired-state tests without `sleep`.
    #[cfg(test)]
    pub async fn insert_with_expires_at(
        &self,
        owner_tenant: String,
        mime_type: String,
        data: Bytes,
        expires_at: DateTime<Utc>,
    ) -> ArtifactHandle {
        let handle = ArtifactHandle::new();
        let meta = ArtifactMeta {
            handle: handle.clone(),
            owner_tenant,
            mime_type,
            size_bytes: data.len() as u64,
            expires_at,
            created_at: Utc::now(),
        };
        self.inner
            .write()
            .await
            .insert(handle.clone(), (meta, data));
        handle
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> InMemoryArtifactStore {
        InMemoryArtifactStore::new(1024 * 1024) // 1 MiB limit
    }

    fn png_bytes(n: usize) -> Bytes {
        Bytes::from(vec![0u8; n])
    }

    /// Plausible defect: get() returns different bytes than what was stored.
    #[tokio::test]
    async fn store_and_retrieve() {
        let s = store();
        let data = png_bytes(128);
        let handle = s
            .store("tenant-a".into(), "image/png".into(), data.clone(), 3600)
            .await
            .unwrap();
        let (meta, retrieved) = s.get(&handle, "tenant-a").await.unwrap();
        assert_eq!(retrieved, data);
        assert_eq!(meta.size_bytes, 128);
        assert_eq!(meta.mime_type, "image/png");
    }

    /// Plausible defect: tenant B can read tenant A's artifact (no isolation).
    #[tokio::test]
    async fn tenant_isolation() {
        let s = store();
        let handle = s
            .store("tenant-a".into(), "image/png".into(), png_bytes(16), 3600)
            .await
            .unwrap();
        let err = s.get(&handle, "tenant-b").await.unwrap_err();
        assert!(
            matches!(err, ArtifactError::Forbidden),
            "expected Forbidden, got {err}"
        );
    }

    /// Plausible defect: expired artifacts are served without error.
    ///
    /// Uses `insert_with_expires_at` with a past timestamp — no sleep needed.
    #[tokio::test]
    async fn expired_artifact() {
        let s = store();
        let past = Utc::now() - Duration::seconds(1);
        let handle = s
            .insert_with_expires_at("tenant-a".into(), "image/png".into(), png_bytes(16), past)
            .await;
        let err = s.get(&handle, "tenant-a").await.unwrap_err();
        assert!(
            matches!(err, ArtifactError::Expired(_)),
            "expected Expired, got {err}"
        );
    }

    /// Plausible defect: oversized payload is silently stored and served.
    #[tokio::test]
    async fn size_limit_enforced() {
        let s = InMemoryArtifactStore::new(64);
        let err = s
            .store("t".into(), "image/png".into(), png_bytes(65), 60)
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                ArtifactError::TooLarge {
                    size: 65,
                    limit: 64
                }
            ),
            "expected TooLarge, got {err}"
        );
    }

    /// Plausible defect: purge_expired keeps expired entries or removes live ones.
    #[tokio::test]
    async fn purge_removes_expired() {
        let s = store();
        // One live artifact.
        s.store("t".into(), "image/png".into(), png_bytes(8), 3600)
            .await
            .unwrap();
        // One already-expired artifact inserted directly.
        s.insert_with_expires_at(
            "t".into(),
            "image/png".into(),
            png_bytes(8),
            Utc::now() - Duration::seconds(1),
        )
        .await;
        let removed = s.purge_expired().await;
        assert_eq!(removed, 1, "exactly one expired artifact must be purged");
    }

    /// Plausible defect: get() panics or errors instead of returning NotFound.
    #[tokio::test]
    async fn handle_not_found() {
        let s = store();
        let handle = ArtifactHandle::new();
        let err = s.get(&handle, "anyone").await.unwrap_err();
        assert!(
            matches!(err, ArtifactError::NotFound(_)),
            "expected NotFound, got {err}"
        );
    }
}

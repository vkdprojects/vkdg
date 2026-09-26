//! In-memory fact store with TTL and tenant isolation.

use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryRecord {
    pub id: Uuid,
    pub tenant_id: String,
    pub session_id: Option<String>,
    pub fact: String,
    pub source: String,  // "conversation" | "explicit"
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    /// Tags for keyword retrieval: e.g. ["language:python", "topic:debugging"]
    pub tags: Vec<String>,
}

impl MemoryRecord {
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            None => false,
            Some(exp) => Utc::now() > exp,
        }
    }
}

pub struct MemoryStore {
    records: RwLock<Vec<MemoryRecord>>,
    max_per_tenant: usize,
}

impl MemoryStore {
    pub fn new(max_per_tenant: usize) -> Arc<Self> {
        Arc::new(Self {
            records: RwLock::new(Vec::new()),
            max_per_tenant,
        })
    }

    /// Store a memory record for a tenant.
    /// If the tenant already has max_per_tenant records, remove the oldest.
    pub async fn store(&self, record: MemoryRecord) {
        let mut records = self.records.write().await;
        // Purge expired entries first
        records.retain(|r| !r.is_expired());
        // If tenant is at capacity, remove oldest
        let tenant_count = records.iter().filter(|r| r.tenant_id == record.tenant_id).count();
        if tenant_count >= self.max_per_tenant {
            if let Some(pos) = records.iter().position(|r| r.tenant_id == record.tenant_id) {
                records.remove(pos);
            }
        }
        records.push(record);
    }

    /// Retrieve relevant memories for a tenant.
    /// Phase D: keyword match on fact text.
    /// Phase E: vector similarity search.
    pub async fn retrieve(&self, tenant_id: &str, query: &str, limit: usize) -> Vec<MemoryRecord> {
        let records = self.records.read().await;
        let query_lower = query.to_lowercase();
        records.iter()
            .filter(|r| r.tenant_id == tenant_id && !r.is_expired())
            .filter(|r| r.fact.to_lowercase().contains(&query_lower)
                     || r.tags.iter().any(|t| query_lower.contains(t.as_str())))
            .take(limit)
            .cloned()
            .collect()
    }

    pub async fn count(&self, tenant_id: &str) -> usize {
        let records = self.records.read().await;
        records.iter().filter(|r| r.tenant_id == tenant_id && !r.is_expired()).count()
    }
}

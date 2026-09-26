use parking_lot::RwLock;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Viewer,
    Operator,
    Admin,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub session_id: String,
    pub user_id: String,
    pub role: Role,
}

pub struct SessionStore {
    sessions: RwLock<HashMap<String, Session>>,
    bootstrap_token: String,
    bootstrap_used: RwLock<bool>,
}

impl SessionStore {
    pub fn new(bootstrap_token: String) -> Arc<Self> {
        Arc::new(Self {
            sessions: RwLock::new(HashMap::new()),
            bootstrap_token,
            bootstrap_used: RwLock::new(false),
        })
    }

    /// Exchange bootstrap token for an admin session. One-time use.
    pub fn bootstrap_login(&self) -> Result<Session, &'static str> {
        let mut used = self.bootstrap_used.write();
        if *used {
            return Err("bootstrap token already used");
        }
        *used = true;
        let session = Session {
            session_id: Uuid::new_v4().to_string(),
            user_id: "admin".to_string(),
            role: Role::Admin,
        };
        self.sessions
            .write()
            .insert(session.session_id.clone(), session.clone());
        Ok(session)
    }

    /// Validate a session cookie value. Returns the session or None.
    pub fn get(&self, session_id: &str) -> Option<Session> {
        self.sessions.read().get(session_id).cloned()
    }

    /// Revoke a session.
    pub fn revoke(&self, session_id: &str) {
        self.sessions.write().remove(session_id);
    }

    pub fn bootstrap_token(&self) -> &str {
        &self.bootstrap_token
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ClientKey {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub scopes: Vec<String>,
    #[serde(skip)]
    // Phase E: hash before storage
    pub raw_token: String,
}

pub struct KeyStore {
    keys: RwLock<Vec<ClientKey>>,
}

impl KeyStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            keys: RwLock::new(Vec::new()),
        })
    }

    /// Create a new key. Returns (ClientKey metadata, raw_token).
    pub fn create(&self, name: String, role: Role, scopes: Vec<String>) -> (ClientKey, String) {
        let raw = Uuid::new_v4().to_string();
        let key = ClientKey {
            id: Uuid::new_v4().to_string(),
            name,
            role,
            created_at: chrono::Utc::now(),
            last_used_at: None,
            scopes,
            raw_token: raw.clone(),
        };
        self.keys.write().push(key.clone());
        (key, raw)
    }

    pub fn list(&self) -> Vec<ClientKey> {
        self.keys.read().clone()
    }

    pub fn revoke(&self, id: &str) -> bool {
        let mut keys = self.keys.write();
        let before = keys.len();
        keys.retain(|k| k.id != id);
        keys.len() < before
    }
}

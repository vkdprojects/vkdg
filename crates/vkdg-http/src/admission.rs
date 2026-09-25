use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use vkdg_core::VkdgError;

pub struct AdmissionGuard {
    semaphore: Arc<Semaphore>,
}

impl AdmissionGuard {
    pub fn new(limit: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(limit)),
        }
    }

    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, VkdgError> {
        self.semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| VkdgError::AdmissionRejected {
                reason: "server at capacity".to_string(),
            })
    }
}

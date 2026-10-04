//! Dynamic model catalog backed by the SQLite `provider_catalog` table.

use std::sync::Arc;

use futures::future::BoxFuture;
use vkdg_config::GatewayStore;
use vkdg_connections::{ConnectionConfig, Credential};

use crate::{ModelCatalog, ProviderError};

/// A [`ModelCatalog`] implementation that reads from the gateway's SQLite store.
///
/// Instantiate once per provider and store behind an `Arc` on the adapter struct.
pub struct DynamicModelCatalog {
    pub store: Arc<GatewayStore>,
    pub provider_id: &'static str,
}

impl ModelCatalog for DynamicModelCatalog {
    fn list_models<'a>(
        &'a self,
        _config: &'a ConnectionConfig,
        _credential: &'a Credential,
    ) -> BoxFuture<'a, Result<Vec<String>, ProviderError>> {
        Box::pin(async move {
            let models = self
                .store
                .catalog_list(self.provider_id)
                .map_err(|e| ProviderError::Config(e.to_string()))?;
            if models.is_empty() {
                return Err(ProviderError::Config(format!(
                    "no models in catalog for provider '{}' — populate via admin API",
                    self.provider_id
                )));
            }
            Ok(models.into_iter().map(|m| m.id).collect())
        })
    }
}

//! Live model discovery.
//!
//! Connections declare route *patterns* (`claude-*`), which `/v1/models` cannot
//! list. A provider that can enumerate what an account may call implements
//! [`ModelCatalog`]; the gateway stores the concrete ids on the connection.

use futures::future::BoxFuture;
use vkdg_connections::{ConnectionConfig, Credential};

use crate::ProviderError;

/// A provider that can list the models one account is allowed to use.
///
/// Returned from [`ProviderAdapter::model_catalog`](crate::ProviderAdapter::model_catalog).
/// The call is made with a single account's credential; nothing is shared
/// between accounts.
pub trait ModelCatalog: Send + Sync {
    /// Concrete model ids in the provider's own spelling (Kiro: `claude-sonnet-4.6`,
    /// Anthropic: `claude-sonnet-4-6`). Never a glob. An empty list is a valid
    /// answer; the caller decides whether to trust it.
    fn list_models<'a>(
        &'a self,
        config: &'a ConnectionConfig,
        credential: &'a Credential,
    ) -> BoxFuture<'a, Result<Vec<String>, ProviderError>>;
}

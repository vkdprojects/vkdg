//! Combos: named routing plans with their own strategy, compression, and cache policy.
//!
//! A combo is the unit of UX. Users configure "coding-fast" or "quality-first"
//! and forget about individual providers. The resolver maps a combo name or
//! model-ID pattern to a concrete routing context before the pipeline runs.

pub mod plan;
pub mod resolver;
pub mod service;
pub mod store;

pub use plan::{BudgetPolicy, CachePolicy, Combo, CompressionPolicy};
pub use resolver::{combo_route_id, ComboResolver};
pub use service::{ComboError, ComboService};
pub use store::ComboStore;
/// Re-exported so combo callers need no direct routing dependency.
pub use vkdg_routing::StrategyKind;

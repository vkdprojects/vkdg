//! Combos: named routing plans with their own strategy, compression, and cache policy.
//!
//! A combo is the unit of UX. Users configure "coding-fast" or "quality-first"
//! and forget about individual providers. The resolver maps a combo name or
//! model-ID pattern to a concrete routing context before the pipeline runs.

pub mod plan;
pub mod resolver;

pub use plan::{BudgetPolicy, CachePolicy, Combo, CompressionPolicy};
pub use resolver::ComboResolver;

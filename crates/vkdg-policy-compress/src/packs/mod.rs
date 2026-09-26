//! Pack module — built-in FilterPack implementations.
//!
//! Each sub-module handles one content class. Import the individual types to
//! register custom subsets, or call `PackRegistry::with_all_defaults()` for
//! the full canonical set.

pub mod command;
pub mod diff;
pub mod docker;
pub mod file_list;
pub mod generic;
pub mod git;
pub mod hex;
pub mod json;
pub mod lint;
pub mod stack_trace;
pub mod tests;
pub mod typescript;

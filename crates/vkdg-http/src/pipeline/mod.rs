//! Request pipeline orchestration.
//!
//! Entry point: run_conversation_pipeline. The pipeline runs each
//! request through admission, routing, credential fetch, upstream
//! dispatch, and response handling as a sequence of named steps.

pub(crate) mod entry;
pub(crate) mod helpers;
pub(crate) mod inner;
pub mod phases;

pub use entry::run_conversation_pipeline;

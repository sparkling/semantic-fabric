//! Provider-free performance receipt primitives.
//!
//! This module records observations and their provenance. It does not decide
//! whether an uncontrolled host is authoritative and never invokes a model or
//! external provider.

pub mod compare;
pub mod config;
mod digest;
pub mod format;
pub mod model;
pub mod proc_status;
pub mod stats;
pub mod worker;

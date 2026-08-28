//! Provider-free performance receipt primitives.
//!
//! This module records observations and their provenance. It does not decide
//! whether an uncontrolled host is authoritative and never invokes a model or
//! external provider.

mod bounded_io;
pub mod capture;
pub mod compare;
pub mod config;
mod digest;
pub mod format;
pub mod model;
pub mod paths;
pub mod proc_status;
pub mod producer;
pub mod profile;
pub mod source;
pub mod stats;
pub mod subprocess;
pub mod worker;
pub mod workload_runner;

// SPDX-License-Identifier: MIT OR Apache-2.0

//! Fail-closed authority-state kernel for one controlled capture attempt.
//!
//! This package is independent of every product `sf-*` crate. It implements
//! transactional ordering and exact-result recovery only. Signatures,
//! transport authentication, transparency publication, witness quorums and
//! controlled-runner launch remain external, unavailable ports.
//! Request-kind semantic-digest and positive-evidence verification are also
//! upstream admission responsibilities; this crate must not be exposed as a
//! network service until those adapters exist.
//! Reference-store crash points do not qualify PostgreSQL process, host or
//! storage restart recovery; that fault-injection evidence remains absent.

mod command;
mod decimal;
mod memory;
mod model;
mod planner;
mod postgres;
mod postgres_binding;
mod postgres_state;
mod protocol;
mod request_json;
mod resource_plan;
mod store;

pub use command::*;
pub use memory::{CrashPoint, InMemoryAuthorityStore, ManualServiceClock};
pub use model::*;
pub use postgres::{PostgresAuthorityStore, MIGRATION_0001};
pub use protocol::*;
pub use store::*;

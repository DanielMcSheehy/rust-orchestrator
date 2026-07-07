//! cortex-core — the domain model shared by every Cortex crate.
//!
//! Nothing in here does I/O. Workflows, tasks, runs, events, and the DAG
//! algebra live in this crate so the store, executor, and server all agree
//! on one vocabulary.

pub mod dag;
pub mod event;
pub mod model;

pub use dag::{topo_layers, validate_dag, DagError};
pub use event::CortexEvent;
pub use model::*;

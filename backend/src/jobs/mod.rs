//! Background jobs run inside the backend process.
//!
//! The Rust backend enqueues scan jobs into Redis and the Python agent layer consumes
//! them; these tasks keep that handoff reliable and schedule recurring work.
pub mod recovery;
pub mod scheduler;

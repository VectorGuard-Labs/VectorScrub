/// Background job module.
/// The Rust backend enqueues jobs into Redis; the Python agent layer
/// consumes them. This module may be expanded with Rust-native workers
/// for lightweight tasks (e.g., scheduled rescans, privacy score updates).
pub mod scheduler;

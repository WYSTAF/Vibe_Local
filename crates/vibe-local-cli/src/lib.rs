//! # vibe-local-cli
//!
//! The `vibe-local` binary plus the embeddable Vibe-Link HTTP server and
//! local discovery responder. The library surface is reused by the JNI
//! bridge so Android runs the exact same server code as the PC.
//!
//! Licensed under Apache-2.0.

pub mod commands;
pub mod discovery;
pub mod server;

pub use server::{DEFAULT_PORT, ServerConfig, ServerHandle, start_server};

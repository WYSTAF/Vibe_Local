//! # vibe-local-engine
//!
//! Local GGUF inference management: `llama-server` process supervision with
//! Vulkan/CUDA/CPU backend selection, a deterministic built-in micro-eval
//! fallback engine, idle auto-unload (default 300s), a thermal governor
//! (42°C throttle / 45°C pause) and the 15-second VibeMark speed benchmark.
//!
//! Licensed under Apache-2.0.

pub mod microeval;
pub mod supervisor;
pub mod thermal;
pub mod vibemark;

pub use microeval::{MicroEval, strip_think};
pub use supervisor::{ChatReply, Engine, EngineConfig, EngineStatus, LoadRequest};
pub use thermal::{ThermalGovernor, ThermalState};
pub use vibemark::VibeMarkResult;

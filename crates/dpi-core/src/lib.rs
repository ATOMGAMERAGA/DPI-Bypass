//! `dpi-core` — platform-agnostic core for DPI-Bypass.
//!
//! This crate has no knowledge of Tauri or of privilege escalation. It models
//! strategies, persists profiles, probes reachability, and drives the
//! auto-solver. The GUI (`src-tauri`) and the privileged helper (`dpi-helper`)
//! both build on it.

pub mod engine;
pub mod error;
pub mod network;
pub mod prober;
pub mod profiles;
pub mod strategy;

pub use error::{CoreError, Result};

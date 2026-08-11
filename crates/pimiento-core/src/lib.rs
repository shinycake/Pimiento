//! Pimiento core — projection reducer and domain types.
//!
//! UI-free by construction: this crate must never depend on GPUI. See
//! `docs/architecture.md` for the projection model this crate hosts.
#![forbid(unsafe_code)]

pub mod diff;
pub mod projection;
pub mod replay;
pub mod todos;
pub mod transcript;

//! Pimiento core — projection reducer and domain types.
//!
//! UI-free by construction: this crate must never depend on GPUI. See
//! `PLAN.md` §5.4 for the projection model this crate will host.
#![forbid(unsafe_code)]

pub mod projection;
pub mod replay;
pub mod transcript;

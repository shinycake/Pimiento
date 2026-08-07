//! OMP RPC client — pure protocol library, no UI dependencies.
//!
//! See `PLAN.md` §4 for the wire contract this crate implements.
#![forbid(unsafe_code)]

pub mod client;
pub mod decoder;
pub mod discovery;
pub mod error;
pub mod frames;
pub mod supervisor;

pub use error::RpcError;

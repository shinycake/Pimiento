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
pub mod update;

pub use error::RpcError;
pub use update::{
    OmpUpdateCheck, OmpUpdateInstall, check_omp_update, install_omp_update,
    parse_update_check_output,
};

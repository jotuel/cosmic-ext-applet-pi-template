//! # Pi RPC
//!
//! `pi_rpc` is a crate for building applications that
//! execute `pi --mode rpc`.

pub mod client;
pub mod error;
pub mod process;
pub mod protocol;
pub mod transport;

pub use client::PiClient;
pub use error::PiError;
pub use process::{PiProcess, PiProcessConfig};
pub use protocol::*;
pub use transport::{JsonLinesReader, JsonLinesWriter};

//! wooosh-core — shared Rust engine. Discovery is deliberately NOT implemented
//! here: the native shells own mDNS (a deviation from DESIGN.md §2).

pub mod api;
pub mod conn;
pub mod control;
pub mod engine;
pub mod error;
pub mod identity;
pub mod inet;
pub mod ledger;
pub mod pairing;
pub mod sanitize;
pub mod transport;
pub mod trust;
pub mod wordlist;

pub use api::*;
pub use error::WoooshError;

uniffi::setup_scaffolding!();

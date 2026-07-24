//! wooosh-core — shared Rust engine for the Wooosh file-sharing app.
//!
//! Implements identity, connections (QUIC/TLS 1.3 with key pinning), pairing
//! (QR + SAS), the CBOR control protocol, and the transfer engine per
//! docs/PROTOCOL.md. Discovery is deliberately NOT implemented here — the
//! native shells own mDNS discovery (deviation from DESIGN.md §2).

pub mod api;
pub mod control;
pub mod engine;
pub mod error;
pub mod identity;
pub mod ledger;
pub mod pairing;
pub mod sanitize;
pub mod transport;
pub mod trust;
pub mod wordlist;

pub use api::*;
pub use error::WoooshError;

uniffi::setup_scaffolding!();

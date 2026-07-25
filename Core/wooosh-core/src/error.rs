use thiserror::Error;

/// Public error type crossing the FFI boundary.
#[derive(Debug, Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum WoooshError {
    #[error("core is not started")]
    NotStarted,
    #[error("core is already started")]
    AlreadyStarted,
    #[error("i/o error: {0}")]
    Io(String),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("connect failed: {0}")]
    Connect(String),
    #[error("unknown peer: {0}")]
    UnknownPeer(String),
    #[error("KEY_CHANGED: pinned key for peer differs from the presented key")]
    KeyChanged,
    #[error("QR_KEY_MISMATCH: presented certificate key does not match the QR key")]
    QrKeyMismatch,
    /// Peer is in PairedOnly visibility and we are not paired (§4.1). Shells
    /// should offer to pair rather than report a generic failure.
    #[error("PAIRING_REQUIRED: peer only accepts connections from paired devices")]
    PairingRequired,
    /// No common protocol version (§4.1/§8).
    #[error("VERSION_MISMATCH: no common protocol version with peer")]
    VersionMismatch,
    #[error("pairing failed: {0}")]
    Pairing(String),
    #[error("invalid QR payload: {0}")]
    InvalidQrPayload(String),
    #[error("transfer error: {0}")]
    Transfer(String),
    #[error("unknown transfer: {0}")]
    UnknownTransfer(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// Hole punching never produced a direct path, so the route to the peer
    /// runs through a relay, and at least one file is over the relayed size
    /// limit (DESIGN.md §9.1). Shells should name the limit rather than report
    /// a generic transfer failure: the same files would send fine on a direct
    /// connection, so this is about the route, not the files.
    #[error("RELAY_FILE_TOO_LARGE: no direct path to the peer and a file exceeds the relayed size limit")]
    RelayFileTooLarge,
}

impl From<std::io::Error> for WoooshError {
    fn from(e: std::io::Error) -> Self {
        WoooshError::Io(e.to_string())
    }
}

/// QUIC application-level close codes (u32 varint) used by wooosh/1.
pub mod close_codes {
    pub const VERSION_MISMATCH: u32 = 1;
    pub const PAIRING_REQUIRED: u32 = 2;
    pub const QR_KEY_MISMATCH: u32 = 3;
    pub const KEY_CHANGED: u32 = 4;
    pub const UNTRUSTED_MSG: u32 = 5;
    pub const TOKEN_INVALID: u32 = 6;
    pub const BYE: u32 = 0;
}

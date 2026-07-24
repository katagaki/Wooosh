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
    /// The peer closed the connection with `PAIRING_REQUIRED` (§4.1): it is in
    /// PairedOnly visibility and we are not a paired device. Shells should say
    /// "this device only accepts transfers from paired devices" and offer to
    /// pair, rather than reporting a generic failure.
    #[error("PAIRING_REQUIRED: peer only accepts connections from paired devices")]
    PairingRequired,
    /// The peer closed the connection with `VERSION_MISMATCH` (§4.1/§8): no
    /// common protocol version.
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

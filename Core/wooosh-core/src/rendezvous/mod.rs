//! Internet path through Wooosh Rendezvous (PROTOCOL.md §9): admission,
//! rooms, STUN, hole punching, and the capped relay fallback.

pub mod admission;
pub mod pathsock;
pub mod room;
pub mod session;
pub mod stun;

pub use admission::Admission;
pub use room::{Role, RoomConn, Tier};
pub use session::{establish, NetSession, Options, QuicRole, DEFAULT_STUN};

/// Baked in at build time (`WOOOSH_RENDEZVOUS_URL`) so a release build points
/// at the production Worker and a development build at its own.
pub fn default_url() -> Option<&'static str> {
    option_env!("WOOOSH_RENDEZVOUS_URL").filter(|s| !s.is_empty())
}

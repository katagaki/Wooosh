//! Internet tickets (PROTOCOL.md §9.3). A ticket names a rendezvous room and
//! carries the publisher's key and a single-use token out of band. The token
//! never reaches the rendezvous: only the room ID does.

use crate::error::WoooshError;
use base64::Engine as _;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

/// Matches the QR token TTL (PROTOCOL.md §4.2): a ticket is a capability
/// handed over in the moment, not a bookmark.
pub const TICKET_TTL: Duration = Duration::from_secs(120);

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Wire form, one line, QR- and chat-safe:
/// `wooosh-net:2?nid=<b64 32B>&tok=<b64 32B>&room=<b64 16B>&dn=<name>&exp=<unix>`
#[derive(Debug, Clone, PartialEq)]
pub struct NetTicket {
    pub version: u64,
    /// Publisher's Ed25519 identity key, pinned in the TLS handshake.
    pub node_id: [u8; 32],
    /// Single-use, same rules as the QR token (§4.2 step 3).
    pub token: [u8; 32],
    /// Rendezvous room; the only part of the ticket the Worker learns.
    pub room: [u8; 16],
    /// Unauthenticated, UI label only.
    pub dn: Option<String>,
    pub expires_unix: u64,
}

/// v1 tickets went through iroh; the two paths do not interoperate.
pub const TICKET_VERSION: u64 = 2;

/// Key for candidate and probe MACs (§9.5): derived from the token, so the
/// rendezvous, which never sees the token, cannot forge either.
pub fn room_key(token: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("wooosh room v2", token)
}

fn pct_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn pct_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hexpair = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hexpair, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl NetTicket {
    pub fn encode(&self) -> String {
        let mut s = format!(
            "wooosh-net:{}?nid={}&tok={}&room={}",
            self.version,
            b64().encode(self.node_id),
            b64().encode(self.token),
            b64().encode(self.room)
        );
        if let Some(dn) = &self.dn {
            s.push_str(&format!("&dn={}", pct_encode(dn)));
        }
        s.push_str(&format!("&exp={}", self.expires_unix));
        s
    }

    pub fn parse(s: &str) -> Result<Self, WoooshError> {
        let bad = |m: &str| WoooshError::InvalidQrPayload(format!("ticket: {m}"));
        let rest = s
            .trim()
            .strip_prefix("wooosh-net:")
            .ok_or_else(|| bad("missing scheme"))?;
        let (ver_str, params) = rest.split_once('?').ok_or_else(|| bad("missing params"))?;
        let version: u64 = ver_str.parse().map_err(|_| bad("bad version"))?;
        if version == 1 {
            return Err(bad("this code is from an older version of Wooosh"));
        }
        if version != TICKET_VERSION {
            return Err(bad("unsupported version"));
        }
        let (mut nid, mut tok, mut room, mut dn, mut exp) = (None, None, None, None, None);
        for kv in params.split(['&', '?']) {
            let Some((k, v)) = kv.split_once('=') else { continue };
            match k {
                "nid" => {
                    let b = b64().decode(v).map_err(|_| bad("nid not base64"))?;
                    nid = Some(<[u8; 32]>::try_from(b.as_slice()).map_err(|_| bad("nid length"))?);
                }
                "tok" => {
                    let b = b64().decode(v).map_err(|_| bad("tok not base64"))?;
                    tok = Some(<[u8; 32]>::try_from(b.as_slice()).map_err(|_| bad("tok length"))?);
                }
                "room" => {
                    let b = b64().decode(v).map_err(|_| bad("room not base64"))?;
                    room = Some(<[u8; 16]>::try_from(b.as_slice()).map_err(|_| bad("room length"))?);
                }
                "dn" => dn = pct_decode(v),
                "exp" => exp = Some(v.parse::<u64>().map_err(|_| bad("bad exp"))?),
                _ => {} // forward compat (PROTOCOL.md §8)
            }
        }
        Ok(NetTicket {
            version,
            node_id: nid.ok_or_else(|| bad("missing nid"))?,
            token: tok.ok_or_else(|| bad("missing tok"))?,
            room: room.ok_or_else(|| bad("missing room"))?,
            dn,
            expires_unix: exp.ok_or_else(|| bad("missing exp"))?,
        })
    }

    pub fn is_expired(&self) -> bool {
        now_unix() > self.expires_unix
    }
}

/// Publisher-side state for an outstanding ticket: single-use, TTL-bound and
/// compared in constant time, like the QR token (PROTOCOL.md §4.2 step 3).
/// Expiry is what stops a ticket pasted into a chat months ago from working.
pub struct TicketPending {
    token: [u8; 32],
    issued_at: Instant,
    used: bool,
}

impl TicketPending {
    pub fn new() -> (Self, [u8; 32]) {
        use rand::RngCore;
        let mut token = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut token);
        (Self { token, issued_at: Instant::now(), used: false }, token)
    }

    pub fn redeem(&mut self, presented: &[u8]) -> bool {
        if self.used || self.issued_at.elapsed() > TICKET_TTL {
            return false;
        }
        if presented.len() != 32 {
            return false;
        }
        let ok = presented.ct_eq(&self.token).unwrap_u8() == 1;
        if ok {
            self.used = true;
        }
        ok
    }
}

pub fn new_expiry_unix() -> u64 {
    now_unix() + TICKET_TTL.as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> NetTicket {
        NetTicket {
            version: TICKET_VERSION,
            node_id: [9; 32],
            token: [4; 32],
            room: [7; 16],
            dn: Some("Kata's iPhone & Co".into()),
            expires_unix: new_expiry_unix(),
        }
    }

    #[test]
    fn ticket_roundtrip() {
        let t = sample();
        let s = t.encode();
        assert!(s.starts_with("wooosh-net:2?nid="));
        assert!(!s.contains(' '));
        assert_eq!(NetTicket::parse(&s).unwrap(), t);
        assert!(!t.is_expired());
    }

    #[test]
    fn ticket_carries_no_addresses() {
        // v1 disclosed LAN and public IPs to anyone the code was forwarded to (§9.3).
        let s = sample().encode();
        assert!(!s.contains("addrs=") && !s.contains("relay="));
    }

    #[test]
    fn ticket_expiry_is_enforced_by_parsing_callers() {
        let t = NetTicket { expires_unix: 1, ..sample() };
        assert!(NetTicket::parse(&t.encode()).unwrap().is_expired());
    }

    #[test]
    fn ticket_rejects_foreign_old_and_malformed_payloads() {
        assert!(NetTicket::parse("wooosh-pair:1?pk=AAAA&tok=BBBB&exp=1").is_err());
        let v1 = NetTicket::parse("wooosh-net:1?nid=AAAA&tok=BBBB&exp=1").unwrap_err();
        assert!(v1.to_string().contains("older version"));
        assert!(NetTicket::parse("wooosh-net:3?nid=AAAA&tok=BBBB&exp=1").is_err());
        let no_room = format!("wooosh-net:2?nid={}&tok={}&exp=1", b64().encode([0u8; 32]), b64().encode([0u8; 32]));
        assert!(NetTicket::parse(&no_room).is_err());
        let short = format!(
            "wooosh-net:2?nid={}&tok={}&room={}&exp=1",
            b64().encode([0u8; 8]),
            b64().encode([0u8; 32]),
            b64().encode([0u8; 16])
        );
        assert!(NetTicket::parse(&short).is_err());
    }

    #[test]
    fn ticket_token_is_single_use() {
        let (mut p, token) = TicketPending::new();
        assert!(!p.redeem(&[0u8; 32]));
        assert!(p.redeem(&token));
        assert!(!p.redeem(&token));
    }

    #[test]
    fn room_key_depends_on_the_token_only() {
        assert_eq!(room_key(&[1; 32]), room_key(&[1; 32]));
        assert_ne!(room_key(&[1; 32]), room_key(&[2; 32]));
    }
}

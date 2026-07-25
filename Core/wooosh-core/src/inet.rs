//! Internet path: iroh endpoint + tickets (PROTOCOL.md §9, DESIGN.md §9.1).
//!
//! The iroh node secret key **is** the Wooosh identity key (PROTOCOL.md §2),
//! so an iroh `EndpointId` is byte-for-byte the public key already in the
//! trust store. A peer paired on the LAN is therefore authenticated over the
//! internet with no extra ceremony, and `device_id_for` /
//! `fingerprint_phrase_for` render the same DeviceID and phrase on both paths.
//!
//! The endpoint is bound **lazily**, on the first ticket operation. Binding it
//! at `start` would have every Wooosh install contact n0's relay servers on
//! launch, which is wrong for a LAN-first app that promises no servers.

use crate::error::WoooshError;
use crate::identity::Identity;
use crate::transport::ALPN;
use base64::Engine as _;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

/// Ticket lifetime. Matches the QR token TTL (PROTOCOL.md §4.2): a ticket is
/// a capability handed to one other human in the moment, not a bookmark.
pub const TICKET_TTL: Duration = Duration::from_secs(120);

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A redeemable internet ticket (PROTOCOL.md §9.2).
///
/// Wire form, one line, QR- and chat-safe:
/// `wooosh-net:1?nid=<b64 32B>&tok=<b64 32B>&dn=<name>&relay=<url>&addrs=<ip:port,…>&exp=<unix>`
///
/// `nid` is the publisher's Ed25519 identity key delivered **out of band**,
/// exactly like `pk` in a pairing QR — which is what makes the internet path
/// MITM-proof without any additional ceremony.
#[derive(Debug, Clone, PartialEq)]
pub struct NetTicket {
    pub version: u64,
    /// Publisher's Ed25519 identity key == iroh EndpointId.
    pub node_id: [u8; 32],
    /// Single-use pairing token, same rules as the QR token (§4.2 step 3).
    pub token: [u8; 32],
    /// Display-name hint (unauthenticated, UI label only).
    pub dn: Option<String>,
    /// Home relay URL, so the redeemer can reach the publisher before any
    /// hole punch succeeds.
    pub relay: Option<String>,
    /// Directly reachable `ip:port` candidates, best-effort.
    pub direct: Vec<String>,
    pub expires_unix: u64,
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
            "wooosh-net:{}?nid={}&tok={}",
            self.version,
            b64().encode(self.node_id),
            b64().encode(self.token)
        );
        if let Some(dn) = &self.dn {
            s.push_str(&format!("&dn={}", pct_encode(dn)));
        }
        if let Some(r) = &self.relay {
            s.push_str(&format!("&relay={}", pct_encode(r)));
        }
        if !self.direct.is_empty() {
            s.push_str(&format!("&addrs={}", self.direct.join(",")));
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
        if version != 1 {
            return Err(bad("unsupported version"));
        }
        let (mut nid, mut tok, mut dn, mut relay, mut exp) = (None, None, None, None, None);
        let mut direct = Vec::new();
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
                "dn" => dn = pct_decode(v),
                "relay" => relay = pct_decode(v).filter(|r| !r.is_empty()),
                "addrs" => {
                    direct = v.split(',').filter(|a| !a.is_empty()).map(String::from).collect()
                }
                "exp" => exp = Some(v.parse::<u64>().map_err(|_| bad("bad exp"))?),
                _ => {} // forward compat (PROTOCOL.md §8)
            }
        }
        Ok(NetTicket {
            version,
            node_id: nid.ok_or_else(|| bad("missing nid"))?,
            token: tok.ok_or_else(|| bad("missing tok"))?,
            dn,
            relay,
            direct,
            expires_unix: exp.ok_or_else(|| bad("missing exp"))?,
        })
    }

    pub fn is_expired(&self) -> bool {
        now_unix() > self.expires_unix
    }

    /// The iroh dial target this ticket describes.
    pub fn endpoint_addr(&self) -> Result<iroh::EndpointAddr, WoooshError> {
        let id = iroh::PublicKey::from_bytes(&self.node_id)
            .map_err(|e| WoooshError::InvalidQrPayload(format!("ticket: bad node id: {e}")))?;
        let mut addr = iroh::EndpointAddr::new(id);
        if let Some(r) = &self.relay {
            let url: iroh::RelayUrl = r
                .parse()
                .map_err(|e| WoooshError::InvalidQrPayload(format!("ticket: bad relay url: {e}")))?;
            addr = addr.with_relay_url(url);
        }
        for a in &self.direct {
            if let Ok(sa) = a.parse::<std::net::SocketAddr>() {
                addr = addr.with_ip_addr(sa);
            }
        }
        Ok(addr)
    }
}

/// Publisher-side state for an outstanding ticket.
///
/// A ticket is a capability: it is single-use, TTL-bound and compared in
/// constant time, exactly like the QR token (PROTOCOL.md §4.2 step 3). Expiry
/// is what stops a ticket pasted into a chat months ago from silently
/// connecting a stranger.
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

/// Bind an iroh endpoint on the Wooosh identity key.
///
/// The key mapping is the whole point of choosing iroh (DESIGN.md §9.1): our
/// Ed25519 secret becomes the node secret, so `endpoint.id()` equals
/// `identity.public_key_bytes()` and every existing pin keeps working.
///
/// `relays` mirrors `Config.relay_urls`:
/// - `None` — n0's free public relays (the default DESIGN.md §9.1 promises).
/// - `Some(&[])` — no relays and no address lookup: direct/hole-punched
///   connections only, from the addresses in the ticket. Nothing leaves the
///   local network unless the ticket says so.
/// - `Some(urls)` — a self-hosted or chosen relay set.
pub async fn bind_endpoint(
    identity: &Identity,
    relays: Option<&[String]>,
) -> Result<iroh::Endpoint, WoooshError> {
    let secret = iroh::SecretKey::from_bytes(&identity.secret_bytes());
    let mut builder = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .secret_key(secret)
        .alpns(vec![ALPN.to_vec()]);
    match relays {
        None => {}
        Some([]) => {
            builder = builder
                .relay_mode(iroh::RelayMode::Disabled)
                .clear_address_lookup();
        }
        Some(urls) => {
            let parsed = urls
                .iter()
                .map(|u| {
                    u.parse::<iroh::RelayUrl>()
                        .map_err(|e| WoooshError::InvalidArgument(format!("relay url {u}: {e}")))
                })
                .collect::<Result<Vec<_>, _>>()?;
            builder = builder.relay_mode(iroh::RelayMode::custom(parsed));
        }
    }
    let ep = builder
        .bind()
        .await
        .map_err(|e| WoooshError::Io(format!("iroh bind: {e}")))?;
    debug_assert_eq!(ep.id().as_bytes(), &identity.public_key_bytes());
    Ok(ep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_roundtrip() {
        let t = NetTicket {
            version: 1,
            node_id: [9; 32],
            token: [4; 32],
            dn: Some("Kata's iPhone & Co".into()),
            relay: Some("https://euw1-1.relay.iroh.network./".into()),
            direct: vec!["192.168.1.5:41234".into(), "[fd00::1]:41234".into()],
            expires_unix: new_expiry_unix(),
        };
        let s = t.encode();
        assert!(s.starts_with("wooosh-net:1?nid="));
        assert!(!s.contains(' '));
        assert_eq!(NetTicket::parse(&s).unwrap(), t);
        assert!(!t.is_expired());
    }

    #[test]
    fn ticket_expiry_is_enforced_by_parsing_callers() {
        let t = NetTicket {
            version: 1,
            node_id: [1; 32],
            token: [2; 32],
            dn: None,
            relay: None,
            direct: vec![],
            expires_unix: 1,
        };
        assert!(NetTicket::parse(&t.encode()).unwrap().is_expired());
    }

    #[test]
    fn ticket_rejects_foreign_and_malformed_payloads() {
        assert!(NetTicket::parse("wooosh-pair:1?pk=AAAA&tok=BBBB&exp=1").is_err());
        assert!(NetTicket::parse("wooosh-net:2?nid=AAAA&tok=BBBB&exp=1").is_err());
        assert!(NetTicket::parse("wooosh-net:1?tok=BBBB&exp=1").is_err());
        // nid must be exactly 32 bytes.
        let short = format!("wooosh-net:1?nid={}&tok={}&exp=1", b64().encode([0u8; 8]), b64().encode([0u8; 32]));
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
    fn ticket_node_id_is_the_identity_key() {
        // The property the whole internet path rests on: the value in the
        // ticket is the same 32 bytes the trust store pins.
        let id = Identity::generate();
        let sk = iroh::SecretKey::from_bytes(&id.secret_bytes());
        assert_eq!(sk.public().as_bytes(), &id.public_key_bytes());
        let t = NetTicket {
            version: 1,
            node_id: id.public_key_bytes(),
            token: [0; 32],
            dn: None,
            relay: None,
            direct: vec![],
            expires_unix: new_expiry_unix(),
        };
        assert_eq!(t.endpoint_addr().unwrap().id.as_bytes(), &id.public_key_bytes());
        assert_eq!(
            crate::identity::device_id_string_for(&t.node_id),
            id.device_id_string()
        );
    }
}

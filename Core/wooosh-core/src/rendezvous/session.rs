//! From an admitted room to a QUIC connection (PROTOCOL.md §9.5–§9.7).

use super::pathsock::{canonical, PathSocket};
use super::room::{RoomConn, RoomEvent};
use crate::error::WoooshError;
use crate::identity::Identity;
use crate::transport;
use serde_json::{json, Value};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

const GATHER_BUDGET: Duration = Duration::from_secs(2);
const CANDIDATE_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(10);
/// Punching normally lands within a few round trips (§9.5 step 3).
const PUNCH_WINDOW: Duration = Duration::from_secs(5);
/// A relayed session keeps probing this long for a direct path (§9.5).
const UPGRADE_WINDOW: Duration = Duration::from_secs(15);
const PROBE_INTERVAL: Duration = Duration::from_millis(100);
const QUIC_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);

pub const DEFAULT_STUN: &str = "stun.cloudflare.com:3478";

/// Which side dials QUIC.
#[derive(Clone, Copy)]
pub enum QuicRole {
    Client { expected_key: Option<[u8; 32]> },
    Server,
    /// Bluetooth and paired rooms: the side with the lower `ufrag` dials, so
    /// both agree without a prior relationship.
    ByUfrag { expected_key: Option<[u8; 32]> },
}

pub struct Options {
    /// `None` skips server-reflexive candidates (tests, or no STUN reachable).
    pub stun: Option<String>,
    /// How long to wait for the other occupant.
    pub wait_for_peer: Duration,
    /// Tests only: never validate a direct path, so the relay carries everything.
    pub force_relay: bool,
}

pub struct NetSession {
    pub conn: quinn::Connection,
    socket: Arc<PathSocket>,
    _endpoint: quinn::Endpoint,
    _room: RoomConn,
}

impl NetSession {
    pub fn is_direct(&self) -> bool {
        self.socket.is_direct()
    }
}

#[derive(Clone)]
struct Candidate {
    kind: &'static str,
    addr: SocketAddr,
}

fn cand_mac(key: &[u8; 32], ufrag: &str, cands: &[(String, String)]) -> String {
    let mut h = blake3::Hasher::new_keyed(key);
    h.update(b"wooosh cand\0");
    h.update(ufrag.as_bytes());
    for (k, a) in cands {
        h.update(b"\n");
        h.update(k.as_bytes());
        h.update(b"|");
        h.update(a.as_bytes());
    }
    hex::encode(&h.finalize().as_bytes()[..16])
}

/// Loopback is included, last: it is how two sessions on one machine meet,
/// and probes are MAC'd, so a peer elsewhere can never validate it.
fn host_candidates(port: u16, ipv6: bool) -> Vec<Candidate> {
    let Ok(ifs) = if_addrs::get_if_addrs() else { return Vec::new() };
    let mut out: Vec<Candidate> = ifs
        .into_iter()
        .map(|i| i.ip())
        .filter(|ip| !ip.is_unspecified())
        .filter(|ip| match ip {
            IpAddr::V4(v4) => !v4.is_link_local(),
            // Link-local needs a scope ID a peer cannot know.
            IpAddr::V6(v6) => ipv6 && (v6.segments()[0] & 0xffc0) != 0xfe80,
        })
        .map(|ip| Candidate { kind: "host", addr: SocketAddr::new(ip, port) })
        .collect();
    out.sort_by_key(|c| c.addr.ip().is_loopback());
    out.dedup_by_key(|c| c.addr);
    out
}

async fn srflx_candidates(sock: &Arc<PathSocket>, stun: &str) -> Vec<Candidate> {
    let Ok(servers) = tokio::net::lookup_host(stun).await else { return Vec::new() };
    let tasks: Vec<_> = servers
        .map(|s| {
            let sock = sock.clone();
            tokio::spawn(async move { sock.stun(s).await })
        })
        .collect();
    let mut out = Vec::new();
    for t in tasks {
        if let Ok(Some(a)) = t.await {
            out.push(Candidate { kind: "srflx", addr: a });
        }
    }
    out
}

fn room_error(code: &str, detail: Option<String>) -> WoooshError {
    match code {
        "relay_unavailable" => WoooshError::RelayUnavailable,
        "quota" => WoooshError::RendezvousQuota,
        _ => WoooshError::RendezvousUnavailable(format!("{code}: {}", detail.unwrap_or_default())),
    }
}

async fn next_event(room: &mut RoomConn, deadline: Instant) -> Result<RoomEvent, WoooshError> {
    match tokio::time::timeout_at(deadline, room.events.recv()).await {
        Err(_) => Err(WoooshError::Connect("timed out waiting for the other device".into())),
        Ok(None) => Err(WoooshError::RendezvousUnavailable("room closed".into())),
        Ok(Some(RoomEvent::Error { code, detail })) => Err(room_error(&code, detail)),
        Ok(Some(RoomEvent::Closed { reason })) => Err(match reason.as_str() {
            "peer_left" => WoooshError::Connect("the other device left".into()),
            "expired" => WoooshError::Connect("the code expired".into()),
            r => WoooshError::RendezvousUnavailable(format!("room closed: {r}")),
        }),
        Ok(Some(ev)) => Ok(ev),
    }
}

/// Runs §9.5–§9.7 to completion and returns the established connection.
/// `room_key` authenticates candidates and probes; the rendezvous never sees it.
pub async fn establish(
    mut room: RoomConn,
    identity: &Identity,
    room_key: [u8; 32],
    role: QuicRole,
    opts: Options,
) -> Result<NetSession, WoooshError> {
    let sock = PathSocket::bind().map_err(|e| WoooshError::Io(format!("bind session socket: {e}")))?;
    if !opts.force_relay {
        sock.set_probe_key(room_key);
    }
    room.receive_relay(&sock);

    // 1. Wait for the other occupant.
    let peer_deadline = Instant::now() + opts.wait_for_peer;
    loop {
        match next_event(&mut room, peer_deadline).await? {
            RoomEvent::Peer => break,
            _ => continue,
        }
    }

    // 2. Gather (bounded), then exchange MAC'd candidates.
    let ipv6 = sock.synthetic().is_ipv6();
    let mut cands = host_candidates(sock.port(), ipv6);
    if let Some(stun) = &opts.stun {
        if let Ok(mut s) = tokio::time::timeout(GATHER_BUDGET, srflx_candidates(&sock, stun)).await {
            cands.append(&mut s);
        }
    }
    let ufrag = hex::encode(rand::random::<[u8; 8]>());
    let wire: Vec<(String, String)> = cands.iter().map(|c| (c.kind.to_string(), c.addr.to_string())).collect();
    room.send_json(json!({
        "t": "cand",
        "c": wire.iter().map(|(k, a)| json!({"kind": k, "a": a})).collect::<Vec<_>>(),
        "ufrag": ufrag,
        "mac": cand_mac(&room_key, &ufrag, &wire),
    }));

    let deadline = Instant::now() + CANDIDATE_EXCHANGE_TIMEOUT;
    let mut remote: Option<(String, Vec<SocketAddr>)> = None;
    let mut go = false;
    while remote.is_none() || !go {
        match next_event(&mut room, deadline).await? {
            RoomEvent::Cand(v) => {
                if let Some(r) = verify_candidates(&room_key, &v) {
                    remote = Some(r);
                } else {
                    log::warn!("dropping candidates with a bad MAC");
                }
            }
            RoomEvent::Go => go = true,
            _ => {}
        }
    }
    let (their_ufrag, their_addrs) = remote.unwrap();

    // 3. Punch. Probing continues in the background for the upgrade window.
    let mut validated = sock.validated();
    {
        let sock = sock.clone();
        let addrs = their_addrs.clone();
        let mut done = sock.validated();
        tokio::spawn(async move {
            let stop = Instant::now() + PUNCH_WINDOW + UPGRADE_WINDOW;
            while Instant::now() < stop && done.borrow().is_none() {
                for a in &addrs {
                    sock.probe(*a).await;
                }
                let _ = tokio::time::timeout(PROBE_INTERVAL, done.changed()).await;
            }
        });
    }
    let _ = tokio::time::timeout(PUNCH_WINDOW, validated.wait_for(|v| v.is_some())).await;
    if !sock.is_direct() {
        log::info!("no direct path after {PUNCH_WINDOW:?}; relaying");
        room.send_relay(&sock);
    } else {
        // Delivered relay frames would be harmless, but the room no longer needs to carry anything.
        room.send_json(json!({"t": "path", "direct": true}));
    }

    // 4. QUIC over the path-selecting socket, pinned exactly as on the LAN.
    let (dial, expected_key) = match role {
        QuicRole::Client { expected_key } => (true, expected_key),
        QuicRole::Server => (false, None),
        QuicRole::ByUfrag { expected_key } => (ufrag < their_ufrag, expected_key),
    };
    let server_cfg = if dial { None } else { Some(transport::server_config(identity)?) };
    let endpoint = quinn::Endpoint::new_with_abstract_socket(
        quinn::EndpointConfig::default(),
        server_cfg,
        sock.clone(),
        Arc::new(quinn::TokioRuntime),
    )
    .map_err(|e| WoooshError::Io(format!("session endpoint: {e}")))?;

    let handshake = async {
        if dial {
            let (cfg, _seen) = transport::client_config(identity, expected_key)?;
            let connecting = endpoint
                .connect_with(cfg, sock.synthetic(), "wooosh")
                .map_err(|e| WoooshError::Connect(format!("connect: {e}")))?;
            connecting.await.map_err(|e| {
                if e.to_string().contains("WOOOSH_KEY_MISMATCH") {
                    WoooshError::KeyChanged
                } else {
                    WoooshError::Connect(format!("handshake: {e}"))
                }
            })
        } else {
            let incoming = endpoint
                .accept()
                .await
                .ok_or_else(|| WoooshError::Connect("session endpoint closed".into()))?;
            incoming.await.map_err(|e| WoooshError::Connect(format!("handshake: {e}")))
        }
    };
    // A refused relay surfaces as a room error, which must beat the handshake timeout.
    let conn = tokio::select! {
        r = tokio::time::timeout(QUIC_HANDSHAKE_TIMEOUT, handshake) => {
            r.map_err(|_| WoooshError::Connect("internet handshake timed out".into()))??
        }
        e = async {
            loop {
                match room.events.recv().await {
                    Some(RoomEvent::Error { code, detail }) => return room_error(&code, detail),
                    Some(RoomEvent::Closed { reason }) => {
                        return WoooshError::Connect(format!("rendezvous closed during handshake: {reason}"))
                    }
                    Some(_) => continue,
                    None => return WoooshError::Connect("rendezvous closed during handshake".into()),
                }
            }
        } => return Err(e),
    };
    Ok(NetSession { conn, socket: sock, _endpoint: endpoint, _room: room })
}

fn verify_candidates(key: &[u8; 32], v: &Value) -> Option<(String, Vec<SocketAddr>)> {
    let ufrag = v["ufrag"].as_str()?.to_string();
    let list = v["c"].as_array()?;
    let wire: Vec<(String, String)> = list
        .iter()
        .filter_map(|c| Some((c["kind"].as_str()?.to_string(), c["a"].as_str()?.to_string())))
        .collect();
    if wire.len() != list.len() || v["mac"].as_str()? != cand_mac(key, &ufrag, &wire) {
        return None;
    }
    let addrs = wire.iter().filter_map(|(_, a)| a.parse().ok()).map(canonical).collect();
    Some((ufrag, addrs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_mac_rejects_injection() {
        let key = [3u8; 32];
        let wire = vec![("host".to_string(), "192.0.2.4:5000".to_string())];
        let mac = cand_mac(&key, "abcd", &wire);
        let good = json!({"ufrag": "abcd", "mac": mac, "c": [{"kind": "host", "a": "192.0.2.4:5000"}]});
        assert_eq!(verify_candidates(&key, &good).unwrap().1, vec!["192.0.2.4:5000".parse().unwrap()]);
        // A rendezvous that adds or swaps a candidate cannot recompute the MAC.
        let injected = json!({"ufrag": "abcd", "mac": mac, "c": [{"kind": "host", "a": "198.51.100.66:5000"}]});
        assert!(verify_candidates(&key, &injected).is_none());
        assert!(verify_candidates(&[4u8; 32], &good).is_none());
    }
}

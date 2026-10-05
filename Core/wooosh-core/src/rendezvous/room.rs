//! Client side of one rendezvous room (PROTOCOL.md §9.4): the WebSocket,
//! admission (§9.2), and relay framing (§9.6).

use super::admission::{Admission, Evidence};
use super::pathsock::PathSocket;
use crate::error::WoooshError;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message as Ws;

/// Relay frame: tag byte then `[u16 len][datagram]` repeated.
const RELAY_TAG: u8 = 0x05;
/// Every frame is a billable Durable Object request (§9.6), so datagrams are
/// coalesced: flush at 256 KiB or after 5 ms, whichever comes first.
const RELAY_BATCH_BYTES: usize = 256 * 1024;
const RELAY_BATCH_DELAY: Duration = Duration::from_millis(5);
const ADMIT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Ticket publisher (the sender).
    Pub,
    /// Ticket redeemer (the receiver).
    Redeem,
    /// Bluetooth introduction (§3.4).
    Peer,
    /// Paired rendezvous (§9.9).
    Pair,
}

impl Role {
    fn wire(self) -> &'static str {
        match self {
            Role::Pub => "pub",
            Role::Redeem => "redeem",
            Role::Peer => "peer",
            Role::Pair => "pair",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    App,
    Hardware,
    Unverified,
}

#[derive(Debug, Clone)]
pub enum RoomEvent {
    Peer,
    Cand(Value),
    Go,
    Error { code: String, detail: Option<String> },
    Closed { reason: String },
}

pub struct RoomConn {
    out: mpsc::UnboundedSender<Ws>,
    pub events: mpsc::UnboundedReceiver<RoomEvent>,
    relay_sink: Arc<Mutex<Option<Arc<PathSocket>>>>,
    pub tier: Tier,
}

/// `https://` and `http://` are accepted for convenience and mapped to WebSocket schemes.
fn room_url(base: &str, room: &[u8; 16], role: Role) -> Result<String, WoooshError> {
    use base64::Engine as _;
    let base = base.trim_end_matches('/');
    let ws = if let Some(r) = base.strip_prefix("https://") {
        format!("wss://{r}")
    } else if let Some(r) = base.strip_prefix("http://") {
        format!("ws://{r}")
    } else if base.starts_with("wss://") || base.starts_with("ws://") {
        base.to_string()
    } else {
        return Err(WoooshError::InvalidArgument(format!("rendezvous url {base}: unsupported scheme")));
    };
    let id = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(room);
    Ok(format!("{ws}/v1/room/{id}?role={}", role.wire()))
}

fn unavailable(e: impl std::fmt::Display) -> WoooshError {
    WoooshError::RendezvousUnavailable(e.to_string())
}

fn b64(b: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn unb64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()
}

pub fn encode_relay_frame(datagrams: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + datagrams.iter().map(|d| d.len() + 2).sum::<usize>());
    out.push(RELAY_TAG);
    for d in datagrams {
        out.extend_from_slice(&(d.len() as u16).to_be_bytes());
        out.extend_from_slice(d);
    }
    out
}

pub fn decode_relay_frame(frame: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    if frame.first() != Some(&RELAY_TAG) {
        return out;
    }
    let mut p = 1;
    while p + 2 <= frame.len() {
        let n = u16::from_be_bytes([frame[p], frame[p + 1]]) as usize;
        let Some(d) = frame.get(p + 2..p + 2 + n) else { break };
        out.push(d);
        p += 2 + n;
    }
    out
}

impl RoomConn {
    /// Connects and completes admission. Returns once this client is admitted;
    /// the other occupant may not have arrived yet.
    pub async fn open(base_url: &str, room: &[u8; 16], role: Role, admission: &Admission) -> Result<RoomConn, WoooshError> {
        // One retry: a stale cached token or key credential is cleared by the
        // first failure, and the second attempt attests from scratch.
        match Self::open_once(base_url, room, role, admission).await {
            Err(WoooshError::RendezvousRetry(_)) => match Self::open_once(base_url, room, role, admission).await {
                Err(WoooshError::RendezvousRetry(m)) => Err(WoooshError::RendezvousUnavailable(m)),
                other => other,
            },
            other => other,
        }
    }

    async fn open_once(base_url: &str, room: &[u8; 16], role: Role, admission: &Admission) -> Result<RoomConn, WoooshError> {
        let url = room_url(base_url, room, role)?;
        let (ws, _) = tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(&url))
            .await
            .map_err(|_| unavailable("rendezvous connect timed out"))?
            .map_err(|e| match e {
                // §9.4: a room already holding two, or a redeemer with no publisher waiting.
                tokio_tungstenite::tungstenite::Error::Http(r) if r.status() == 409 => {
                    WoooshError::Pairing("this code has already been used".into())
                }
                tokio_tungstenite::tungstenite::Error::Http(r) if r.status() == 410 => {
                    WoooshError::Pairing("this code has expired or was withdrawn".into())
                }
                e => unavailable(e),
            })?;
        let (mut sink, mut stream) = ws.split();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Ws>();
        tokio::spawn(async move {
            while let Some(m) = out_rx.recv().await {
                let close = matches!(m, Ws::Close(_));
                if sink.send(m).await.is_err() || close {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        // Admission runs on the raw stream before the event pump takes it over.
        let send = |v: Value| {
            let _ = out_tx.send(Ws::Text(v.to_string().into()));
        };
        let deadline = tokio::time::Instant::now() + ADMIT_TIMEOUT;
        let tier = loop {
            let msg = tokio::time::timeout_at(deadline, stream.next())
                .await
                .map_err(|_| unavailable("admission timed out"))?;
            let text = match msg {
                Some(Ok(Ws::Text(t))) => t.to_string(),
                Some(Ok(Ws::Close(f))) => {
                    return Err(unavailable(format!("closed during admission: {}", f.map(|f| f.reason.to_string()).unwrap_or_default())));
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(unavailable(e)),
                None => return Err(unavailable("closed during admission")),
            };
            let v: Value = serde_json::from_str(&text).map_err(unavailable)?;
            match v["t"].as_str() {
                Some("challenge") => {
                    let nonce = v["nonce"].as_str().and_then(unb64).ok_or_else(|| unavailable("bad challenge"))?;
                    let ev = admission.evidence(&nonce).await;
                    send(match ev {
                        Evidence::Token(t) => json!({"t": "admit", "token": t}),
                        Evidence::AppleAttest { key_id, attestation } => {
                            json!({"t": "admit", "apple": {"key_id": b64(&key_id), "attestation": b64(&attestation)}})
                        }
                        Evidence::AppleAssert { key_credential, assertion } => {
                            json!({"t": "admit", "apple": {"key_credential": key_credential, "assertion": b64(&assertion)}})
                        }
                        Evidence::Android { token, install_id } => {
                            json!({"t": "admit", "android": {"token": token, "install_id": b64(&install_id)}})
                        }
                        Evidence::Tpm { ek_cert, ek_chain, ak_public } => json!({"t": "admit", "tpm": {
                            "ek_cert": b64(&ek_cert),
                            "ek_chain": ek_chain.iter().map(|c| b64(c)).collect::<Vec<_>>(),
                            "ak_public": b64(&ak_public),
                        }}),
                        Evidence::Anonymous => json!({"t": "admit", "anon": true}),
                    });
                }
                Some("tpm-challenge") => {
                    let blob = v["credential_blob"].as_str().and_then(unb64).ok_or_else(|| unavailable("bad tpm challenge"))?;
                    let secret = v["encrypted_secret"].as_str().and_then(unb64).ok_or_else(|| unavailable("bad tpm challenge"))?;
                    match admission.tpm_activate(blob, secret).await {
                        Some(s) => send(json!({"t": "tpm-activate", "secret": b64(&s)})),
                        None => return Err(WoooshError::RendezvousRetry("TPM activation failed".into())),
                    }
                }
                Some("admitted") => {
                    admission.remember(v["token"].as_str(), v["key_credential"].as_str());
                    break match v["tier"].as_str() {
                        Some("app") => Tier::App,
                        Some("hardware") => Tier::Hardware,
                        _ => Tier::Unverified,
                    };
                }
                Some("error") => {
                    let code = v["code"].as_str().unwrap_or("").to_string();
                    return Err(match code.as_str() {
                        "quota" => WoooshError::RendezvousQuota,
                        "bad_token" | "attestation_failed" => {
                            admission.forget();
                            WoooshError::RendezvousRetry(format!("{code}: {}", v["detail"].as_str().unwrap_or("")))
                        }
                        _ => unavailable(format!("{code}: {}", v["detail"].as_str().unwrap_or(""))),
                    });
                }
                _ => {}
            }
        };

        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let relay_sink: Arc<Mutex<Option<Arc<PathSocket>>>> = Arc::new(Mutex::new(None));
        let sink_for_pump = relay_sink.clone();
        tokio::spawn(async move {
            let reason = loop {
                match stream.next().await {
                    Some(Ok(Ws::Text(t))) => {
                        let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
                        let ev = match v["t"].as_str() {
                            Some("peer") => RoomEvent::Peer,
                            Some("cand") => RoomEvent::Cand(v),
                            Some("go") => RoomEvent::Go,
                            Some("error") => RoomEvent::Error {
                                code: v["code"].as_str().unwrap_or("").into(),
                                detail: v["detail"].as_str().map(String::from),
                            },
                            _ => continue,
                        };
                        if ev_tx.send(ev).is_err() {
                            break String::new();
                        }
                    }
                    Some(Ok(Ws::Binary(b))) => {
                        let sock = sink_for_pump.lock().unwrap().clone();
                        if let Some(sock) = sock {
                            for d in decode_relay_frame(&b) {
                                sock.deliver_relayed(d.to_vec());
                            }
                        }
                    }
                    Some(Ok(Ws::Close(f))) => break f.map(|f| f.reason.to_string()).unwrap_or_default(),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => break e.to_string(),
                    None => break String::new(),
                }
            };
            let _ = ev_tx.send(RoomEvent::Closed { reason });
        });

        Ok(RoomConn { out: out_tx, events: ev_rx, relay_sink, tier })
    }

    pub fn send_json(&self, v: Value) {
        let _ = self.out.send(Ws::Text(v.to_string().into()));
    }

    /// Relayed datagrams from the peer reach `sock` from now on. Always
    /// wired: the peer may be relaying even when this side found a direct path.
    pub fn receive_relay(&self, sock: &Arc<PathSocket>) {
        *self.relay_sink.lock().unwrap() = Some(sock.clone());
    }

    /// Datagrams with no direct path go out as batched relay frames.
    pub fn send_relay(&self, sock: &Arc<PathSocket>) {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        sock.enable_relay(tx);
        let out = self.out.clone();
        tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut batch = vec![first];
                let mut size = batch[0].len() + 2;
                let flush_at = tokio::time::Instant::now() + RELAY_BATCH_DELAY;
                while size < RELAY_BATCH_BYTES {
                    match tokio::time::timeout_at(flush_at, rx.recv()).await {
                        Ok(Some(d)) => {
                            size += d.len() + 2;
                            batch.push(d);
                        }
                        Ok(None) | Err(_) => break,
                    }
                }
                if out.send(Ws::Binary(encode_relay_frame(&batch).into())).is_err() {
                    return;
                }
            }
        });
    }

    pub fn close(&self) {
        let _ = self.out.send(Ws::Close(None));
    }
}

impl Drop for RoomConn {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_frames_roundtrip_and_tolerate_truncation() {
        let dgrams = vec![vec![1u8, 2, 3], vec![], vec![9u8; 1400]];
        let f = encode_relay_frame(&dgrams);
        let back: Vec<Vec<u8>> = decode_relay_frame(&f).into_iter().map(|d| d.to_vec()).collect();
        assert_eq!(back, dgrams);
        assert_eq!(decode_relay_frame(&f[..f.len() - 1]).len(), 2);
        assert!(decode_relay_frame(&[0x06, 0, 1, 7]).is_empty());
    }

    #[test]
    fn room_urls_accept_http_and_ws_schemes() {
        let id = [0u8; 16];
        assert_eq!(
            room_url("https://r.example.dev/", &id, Role::Pub).unwrap(),
            "wss://r.example.dev/v1/room/AAAAAAAAAAAAAAAAAAAAAA?role=pub"
        );
        assert!(room_url("ws://127.0.0.1:8787", &id, Role::Redeem).unwrap().starts_with("ws://127.0.0.1:8787/v1/room/"));
        assert!(room_url("ftp://x", &id, Role::Pub).is_err());
    }
}

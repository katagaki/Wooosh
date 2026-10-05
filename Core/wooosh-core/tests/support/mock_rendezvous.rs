//! In-process stand-in for Wooosh Rendezvous, implementing the room protocol
//! of PROTOCOL.md §9.4 so the integration suite stays hermetic. Admission is
//! not verified here (the Worker's own suite covers it): `anon` admits as
//! Unverified, anything else as App. The Worker itself is exercised end to end
//! by `internet_ticket_via_local_worker`.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::frame::{coding::CloseCode, CloseFrame};
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone)]
struct Client {
    id: u64,
    role: String,
    admitted: bool,
    tier: &'static str,
    cand: bool,
    relay_refused: bool,
    tx: mpsc::UnboundedSender<Message>,
}

#[derive(Default)]
struct State {
    rooms: HashMap<String, Vec<Client>>,
    next_id: u64,
}

pub struct MockRendezvous {
    pub url: String,
    _rt: tokio::runtime::Runtime,
}

fn compatible(a: &str, b: &str) -> bool {
    matches!((a, b), ("pub", "redeem") | ("redeem", "pub")) || (a == b && a != "pub" && a != "redeem")
}

fn text(v: Value) -> Message {
    Message::Text(v.to_string().into())
}

impl MockRendezvous {
    /// `relay_unverified`: whether anonymous clients may relay (the Worker's
    /// dev environment says yes, production says no).
    pub fn start(relay_unverified: bool) -> MockRendezvous {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build().unwrap();
        let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        rt.spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let state = state.clone();
                tokio::spawn(async move { serve(tcp, state, relay_unverified).await });
            }
        });
        MockRendezvous { url, _rt: rt }
    }
}

async fn serve(tcp: tokio::net::TcpStream, state: Arc<Mutex<State>>, relay_unverified: bool) {
    let mut room = String::new();
    let mut role = String::new();
    let st = state.clone();
    let accepted = tokio_tungstenite::accept_hdr_async(tcp, |req: &Request, resp: Response| {
        let refuse = |code: u16| -> Result<Response, ErrorResponse> {
            let mut r = ErrorResponse::new(None);
            *r.status_mut() = code.try_into().unwrap();
            Err(r)
        };
        let path = req.uri().path().to_string();
        let query = req.uri().query().unwrap_or("").to_string();
        let Some(id) = path.strip_prefix("/v1/room/") else { return refuse(404) };
        let Some(r) = query.strip_prefix("role=") else { return refuse(400) };
        let s = st.lock().unwrap();
        let present = s.rooms.get(id).cloned().unwrap_or_default();
        if present.len() >= 2 || present.iter().any(|c| !compatible(&c.role, r)) {
            return refuse(409);
        }
        if r == "redeem" && !present.iter().any(|c| c.role == "pub") {
            return refuse(410);
        }
        room = id.to_string();
        role = r.to_string();
        Ok(resp)
    })
    .await;
    let Ok(ws) = accepted else { return };
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            let close = matches!(m, Message::Close(_));
            if sink.send(m).await.is_err() || close {
                break;
            }
        }
    });
    let id = {
        let mut s = state.lock().unwrap();
        s.next_id += 1;
        let id = s.next_id;
        s.rooms.entry(room.clone()).or_default().push(Client {
            id,
            role: role.clone(),
            admitted: false,
            tier: "unverified",
            cand: false,
            relay_refused: false,
            tx: tx.clone(),
        });
        id
    };
    let _ = tx.send(text(json!({"t": "challenge", "nonce": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"})));

    while let Some(Ok(msg)) = stream.next().await {
        let mut s = state.lock().unwrap();
        let clients = s.rooms.entry(room.clone()).or_default();
        let Some(me) = clients.iter().position(|c| c.id == id) else { break };
        let other = clients.iter().position(|c| c.id != id);
        match msg {
            Message::Binary(b) => {
                let allowed = |t: &str| t != "unverified" || relay_unverified;
                let Some(o) = other else { continue };
                if !clients[o].admitted {
                    continue;
                }
                if allowed(clients[me].tier) && allowed(clients[o].tier) {
                    let _ = clients[o].tx.send(Message::Binary(b));
                } else if !clients[me].relay_refused {
                    clients[me].relay_refused = true;
                    let _ = tx.send(text(json!({"t": "error", "code": "relay_unavailable"})));
                }
            }
            Message::Text(t) => {
                let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
                match v["t"].as_str() {
                    Some("admit") => {
                        let tier = if v["anon"] == json!(true) { "unverified" } else { "app" };
                        clients[me].admitted = true;
                        clients[me].tier = tier;
                        let _ = tx.send(text(json!({"t": "admitted", "tier": tier})));
                        if let Some(o) = other.filter(|&o| clients[o].admitted) {
                            let _ = tx.send(text(json!({"t": "peer"})));
                            let _ = clients[o].tx.send(text(json!({"t": "peer"})));
                        }
                    }
                    Some("cand") => {
                        clients[me].cand = true;
                        if let Some(o) = other.filter(|&o| clients[o].admitted) {
                            let _ = clients[o].tx.send(text(json!({"t": "cand", "c": v["c"], "ufrag": v["ufrag"], "mac": v["mac"]})));
                            if clients[o].cand {
                                let _ = tx.send(text(json!({"t": "go"})));
                                let _ = clients[o].tx.send(text(json!({"t": "go"})));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    let mut s = state.lock().unwrap();
    if let Some(clients) = s.rooms.get_mut(&room) {
        clients.retain(|c| c.id != id);
        for c in clients.iter() {
            let _ = c.tx.send(Message::Close(Some(CloseFrame { code: CloseCode::Library(4001), reason: "peer_left".into() })));
        }
        clients.clear();
    }
}

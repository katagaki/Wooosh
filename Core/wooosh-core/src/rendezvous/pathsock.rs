//! The UDP socket quinn sees for one internet session (PROTOCOL.md §9.5).
//!
//! quinn is shown a single synthetic peer address for the whole connection.
//! Underneath, each datagram goes over the direct path once a punch probe has
//! validated one, and through the room's relay otherwise. quinn therefore
//! never migrates: a session that starts relayed moves onto a direct path the
//! moment punching succeeds, invisibly to it.
//!
//! One task owns the receive side of the real socket and sorts packets: STUN
//! (server replies and peer probes) is handled here, QUIC goes to quinn.

use super::stun::{self, TxId};
use quinn::udp::{RecvMeta, Transmit};
use quinn::{AsyncUdpSocket, UdpPoller};
use std::collections::HashMap;
use std::fmt;
use std::io::{self, IoSliceMut};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, watch};

/// Packets queued for quinn before the oldest are dropped, as a kernel socket
/// buffer would.
const INBOUND_QUEUE: usize = 4096;

pub struct PathSocket {
    udp: Arc<UdpSocket>,
    local: SocketAddr,
    dual_stack: bool,
    synthetic: SocketAddr,
    direct: Mutex<Option<SocketAddr>>,
    relay_out: Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>,
    inbound_tx: mpsc::Sender<Vec<u8>>,
    inbound_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    stun_waiters: Mutex<HashMap<TxId, oneshot::Sender<SocketAddr>>>,
    probe_key: Mutex<Option<[u8; 32]>>,
    probes_sent: Mutex<HashMap<TxId, SocketAddr>>,
    validated: watch::Sender<Option<SocketAddr>>,
}

impl fmt::Debug for PathSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PathSocket").field("local", &self.local).finish()
    }
}

fn probe_mac(key: &[u8; 32], label: &[u8], txid: &TxId) -> [u8; 16] {
    let mut h = blake3::Hasher::new_keyed(key);
    h.update(label);
    h.update(txid);
    let mut out = [0u8; 16];
    out.copy_from_slice(&h.finalize().as_bytes()[..16]);
    out
}

/// IPv4-mapped IPv6 back to IPv4, so candidates compare equal however they arrived.
pub fn canonical(a: SocketAddr) -> SocketAddr {
    SocketAddr::new(a.ip().to_canonical(), a.port())
}

impl PathSocket {
    /// Binds a dual-stack socket where the OS allows one, IPv4 otherwise.
    pub fn bind() -> io::Result<Arc<Self>> {
        let (std_sock, dual_stack) = match bind_dual_stack() {
            Ok(s) => (s, true),
            Err(_) => (std::net::UdpSocket::bind("0.0.0.0:0")?, false),
        };
        std_sock.set_nonblocking(true)?;
        let udp = Arc::new(UdpSocket::from_std(std_sock)?);
        let local = udp.local_addr()?;
        // Never routed: a ULA (or, IPv4-only, a benchmarking-range address)
        // that quinn treats as its one peer.
        let synthetic = if local.is_ipv6() {
            SocketAddr::new(IpAddr::V6(Ipv6Addr::new(0xfd77, 0x6f6f, 0x7368, 0, 0, 0, 0, 1)), 1)
        } else {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1)), 1)
        };
        let (inbound_tx, inbound_rx) = mpsc::channel(INBOUND_QUEUE);
        let sock = Arc::new(PathSocket {
            udp,
            local,
            dual_stack,
            synthetic,
            direct: Mutex::new(None),
            relay_out: Mutex::new(None),
            inbound_tx,
            inbound_rx: Mutex::new(inbound_rx),
            stun_waiters: Mutex::new(HashMap::new()),
            probe_key: Mutex::new(None),
            probes_sent: Mutex::new(HashMap::new()),
            validated: watch::channel(None).0,
        });
        let weak = Arc::downgrade(&sock);
        let udp = sock.udp.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            loop {
                let (n, from) = match udp.recv_from(&mut buf).await {
                    Ok(v) => v,
                    // ICMP port-unreachable surfaces as a recv error on some OSes; it is not fatal.
                    Err(_) => {
                        if weak.strong_count() == 0 {
                            return;
                        }
                        continue;
                    }
                };
                let Some(sock) = weak.upgrade() else { return };
                sock.on_udp(&buf[..n], canonical(from));
            }
        });
        Ok(sock)
    }

    pub fn port(&self) -> u16 {
        self.local.port()
    }

    pub fn synthetic(&self) -> SocketAddr {
        self.synthetic
    }

    pub fn is_direct(&self) -> bool {
        self.direct.lock().unwrap().is_some()
    }

    pub fn direct_addr(&self) -> Option<SocketAddr> {
        *self.direct.lock().unwrap()
    }

    pub fn validated(&self) -> watch::Receiver<Option<SocketAddr>> {
        self.validated.subscribe()
    }

    pub fn set_probe_key(&self, key: [u8; 32]) {
        *self.probe_key.lock().unwrap() = Some(key);
    }

    /// From here on, datagrams without a direct path go to the relay.
    pub fn enable_relay(&self, tx: mpsc::UnboundedSender<Vec<u8>>) {
        *self.relay_out.lock().unwrap() = Some(tx);
    }

    /// Datagrams unpacked from relay frames.
    pub fn deliver_relayed(&self, datagram: Vec<u8>) {
        let _ = self.inbound_tx.try_send(datagram);
    }

    fn wire(&self, a: SocketAddr) -> SocketAddr {
        match (self.dual_stack, a) {
            (true, SocketAddr::V4(v4)) => SocketAddr::new(IpAddr::V6(v4.ip().to_ipv6_mapped()), v4.port()),
            _ => a,
        }
    }

    pub async fn send_to(&self, packet: &[u8], to: SocketAddr) -> io::Result<()> {
        if !self.dual_stack && to.is_ipv6() {
            return Ok(());
        }
        self.udp.send_to(packet, self.wire(to)).await.map(|_| ())
    }

    /// Asks a STUN server for this socket's public address.
    pub async fn stun(&self, server: SocketAddr) -> Option<SocketAddr> {
        let txid: TxId = rand::random();
        let (tx, rx) = oneshot::channel();
        self.stun_waiters.lock().unwrap().insert(txid, tx);
        let req = stun::binding_request(&txid, None);
        let mut answer = None;
        // Three tries over ~1.5 s: UDP to a STUN server is lossy too.
        let mut rx = rx;
        for _ in 0..3 {
            if self.send_to(&req, server).await.is_err() {
                break;
            }
            match tokio::time::timeout(std::time::Duration::from_millis(500), &mut rx).await {
                Ok(Ok(a)) => {
                    answer = Some(canonical(a));
                    break;
                }
                Ok(Err(_)) => break,
                Err(_) => continue,
            }
        }
        self.stun_waiters.lock().unwrap().remove(&txid);
        answer
    }

    /// One authenticated probe toward a peer candidate.
    pub async fn probe(&self, to: SocketAddr) {
        let Some(key) = *self.probe_key.lock().unwrap() else { return };
        let txid: TxId = rand::random();
        self.probes_sent.lock().unwrap().insert(txid, to);
        let mac = probe_mac(&key, b"wooosh probe req", &txid);
        let _ = self.send_to(&stun::binding_request(&txid, Some(&mac)), to).await;
    }

    fn on_udp(&self, packet: &[u8], from: SocketAddr) {
        if !stun::looks_like_stun(packet) {
            // QUIC. Unauthenticated sources are harmless here: quinn drops
            // anything that does not decrypt.
            let _ = self.inbound_tx.try_send(packet.to_vec());
            return;
        }
        let Some(msg) = stun::Message::parse(packet) else { return };
        let key = *self.probe_key.lock().unwrap();
        match msg.kind {
            stun::BINDING_SUCCESS => {
                if let Some(w) = self.stun_waiters.lock().unwrap().remove(&msg.txid) {
                    if let Some(a) = msg.mapped_address() {
                        let _ = w.send(a);
                    }
                    return;
                }
                let (Some(key), Some(auth)) = (key, msg.attr(stun::ATTR_WOOOSH_AUTH)) else { return };
                if self.probes_sent.lock().unwrap().remove(&msg.txid).is_none() {
                    return;
                }
                if auth != probe_mac(&key, b"wooosh probe resp", &msg.txid) {
                    return;
                }
                // First validated path wins; it carries every datagram from now on.
                let mut direct = self.direct.lock().unwrap();
                if direct.is_none() {
                    *direct = Some(from);
                    self.validated.send_replace(Some(from));
                }
            }
            stun::BINDING_REQUEST => {
                let (Some(key), Some(auth)) = (key, msg.attr(stun::ATTR_WOOOSH_AUTH)) else { return };
                if auth != probe_mac(&key, b"wooosh probe req", &msg.txid) {
                    return;
                }
                let reply = stun::binding_success(&msg.txid, from, &probe_mac(&key, b"wooosh probe resp", &msg.txid));
                let _ = self.udp.try_send_to(&reply, self.wire(from));
            }
            _ => {}
        }
    }
}

fn bind_dual_stack() -> io::Result<std::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_only_v6(false)?;
    s.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0).into())?;
    Ok(s.into())
}

#[derive(Debug)]
struct Poller(Arc<PathSocket>);

impl UdpPoller for Poller {
    fn poll_writable(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        self.0.udp.poll_send_ready(cx)
    }
}

impl AsyncUdpSocket for PathSocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        Box::pin(Poller(self))
    }

    fn try_send(&self, transmit: &Transmit) -> io::Result<()> {
        if let Some(to) = self.direct_addr() {
            return self.udp.try_send_to(transmit.contents, self.wire(to)).map(|_| ());
        }
        if let Some(tx) = self.relay_out.lock().unwrap().as_ref() {
            let _ = tx.send(transmit.contents.to_vec());
        }
        // No path yet: dropped like a packet into a black hole. QUIC retransmits.
        Ok(())
    }

    fn poll_recv(&self, cx: &mut Context, bufs: &mut [IoSliceMut<'_>], meta: &mut [RecvMeta]) -> Poll<io::Result<usize>> {
        let mut rx = self.inbound_rx.lock().unwrap();
        let mut n = 0;
        while n < bufs.len().min(meta.len()) {
            let pkt = if n == 0 {
                match rx.poll_recv(cx) {
                    Poll::Ready(Some(p)) => p,
                    Poll::Ready(None) => return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into())),
                    Poll::Pending => return Poll::Pending,
                }
            } else {
                match rx.try_recv() {
                    Ok(p) => p,
                    Err(_) => break,
                }
            };
            let len = pkt.len().min(bufs[n].len());
            bufs[n][..len].copy_from_slice(&pkt[..len]);
            meta[n] = RecvMeta { addr: self.synthetic, len, stride: len, ecn: None, dst_ip: None };
            n += 1;
        }
        Poll::Ready(Ok(n))
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.local)
    }

    /// Without DF on this socket, MTU probes could be fragmented and appear to
    /// succeed; staying at QUIC's 1200-byte floor is the safe choice.
    fn may_fragment(&self) -> bool {
        true
    }
}

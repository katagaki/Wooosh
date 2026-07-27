//! Everything runs on 127.0.0.1 with no relay, so the suite is hermetic apart
//! from the one `#[ignore]`d public-relay test.

use rand::{RngCore, SeedableRng};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use wooosh_core::control::Msg;
use wooosh_core::{
    fingerprint_phrase_for, parse_pairing_qr, Config, CoreEvent, CoreEventListener, DeviceType,
    FileKeyStore, KeyStore, TransferDirection, Visibility, WoooshCore, WoooshError,
};

const MIB: u64 = 1024 * 1024;

struct Listener(mpsc::Sender<CoreEvent>);

impl CoreEventListener for Listener {
    fn on_event(&self, event: CoreEvent) {
        let _ = self.0.send(event);
    }
}

struct Node {
    core: Arc<WoooshCore>,
    rx: mpsc::Receiver<CoreEvent>,
    dir: PathBuf,
}

impl Node {
    fn start(base: &Path, name: &str, vis: Visibility) -> Node {
        Node::start_with(base, name, vis, "127.0.0.1:0", None)
    }

    /// No relay and no address lookup, so the iroh flow stays on loopback.
    fn start_offline_internet(base: &Path, name: &str, vis: Visibility) -> Node {
        Node::start_with(base, name, vis, "127.0.0.1:0", Some(Vec::new()))
    }

    /// Lets a second identity take over the exact `ip:port` a paired peer held.
    fn start_on(base: &Path, name: &str, vis: Visibility, listen: &str) -> Node {
        Node::start_with(base, name, vis, listen, None)
    }

    fn start_with(
        base: &Path,
        name: &str,
        vis: Visibility,
        listen: &str,
        relay_urls: Option<Vec<String>>,
    ) -> Node {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let ks_path = dir.join("id.key").to_string_lossy().to_string();
        let cfg = Config {
            device_name: name.to_string(),
            device_type: DeviceType::Desktop,
            visibility: vis,
            staging_dir: dir.join("staging").to_string_lossy().to_string(),
            trust_store_path: dir.join("trust.json").to_string_lossy().to_string(),
            listen_addr: Some(listen.to_string()),
            relay_urls,
        };
        // Re-binding a just-released port races the OS reclaiming the socket.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let core = WoooshCore::new();
            let (tx, rx) = mpsc::channel();
            let ks: Arc<dyn KeyStore> = FileKeyStore::new(ks_path.clone());
            match core.start(cfg.clone(), ks, Arc::new(Listener(tx))) {
                Ok(()) => return Node { core, rx, dir },
                Err(e) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100));
                    let _ = e;
                }
                Err(e) => panic!("node {name} failed to start on {listen}: {e}"),
            }
        }
    }

    fn addr(&self) -> String {
        self.core.listen_addr().unwrap()
    }

    fn staging(&self) -> PathBuf {
        self.dir.join("staging")
    }

    /// So a later `wait_for` cannot be satisfied by a stale event.
    fn drain(&self) {
        while self.rx.try_recv().is_ok() {}
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        self.core.stop();
    }
}

fn wait_for<T>(
    rx: &mpsc::Receiver<CoreEvent>,
    timeout: Duration,
    what: &str,
    f: impl Fn(&CoreEvent) -> Option<T>,
) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        let rem = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or_else(|| panic!("timed out waiting for {what}"));
        match rx.recv_timeout(rem) {
            Ok(ev) => {
                if let Some(t) = f(&ev) {
                    return t;
                }
            }
            Err(e) => panic!("timed out waiting for {what}: {e}"),
        }
    }
}

fn write_random_file(path: &Path, size: u64, seed: u64) {
    use std::io::Write;
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut buf = vec![0u8; 4 * MIB as usize];
    let mut remaining = size;
    while remaining > 0 {
        let n = std::cmp::min(remaining, buf.len() as u64) as usize;
        rng.fill_bytes(&mut buf[..n]);
        f.write_all(&buf[..n]).unwrap();
        remaining -= n as u64;
    }
    f.flush().unwrap();
}

fn b3_of(path: &Path) -> String {
    let mut hasher = blake3::Hasher::new();
    let mut f = std::fs::File::open(path).unwrap();
    std::io::copy(&mut f, &mut hasher).unwrap();
    hasher.finalize().to_hex().to_string()
}

#[test]
fn pair_via_qr() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start(tmp.path(), "receiver", Visibility::Everyone);
    let s = Node::start(tmp.path(), "sender", Visibility::Everyone);

    let payload = r.core.begin_pairing_qr().unwrap();
    assert!(payload.starts_with("wooosh-pair:1?pk="));

    let info = parse_pairing_qr(payload.clone()).unwrap();
    assert_eq!(info.device_name.as_deref(), Some("receiver"));
    assert_eq!(info.device_id, r.core.device_id().unwrap());
    assert!(!info.expired);

    let peer_id = s.core.pair_with_qr(payload).unwrap();
    assert_eq!(peer_id, r.core.device_id().unwrap());

    let ok = wait_for(&s.rx, Duration::from_secs(10), "sender PairingResult", |e| match e {
        CoreEvent::PairingResult { success, .. } => Some(*success),
        _ => None,
    });
    assert!(ok);
    let ok = wait_for(&r.rx, Duration::from_secs(10), "receiver PairingResult", |e| match e {
        CoreEvent::PairingResult { success, .. } => Some(*success),
        _ => None,
    });
    assert!(ok);

    let r_pk = r.core.public_key().unwrap();
    let pid2 = s.core.connect_peer(r.addr(), Some(r_pk)).unwrap();
    assert_eq!(pid2, peer_id);
    let trusted = wait_for(
        &r.rx,
        Duration::from_secs(10),
        "trusted PeerConnected on receiver",
        |e| match e {
            CoreEvent::PeerConnected { trusted, .. } if *trusted => Some(true),
            _ => None,
        },
    );
    assert!(trusted);
}

#[test]
fn pair_via_qr_rejects_bad_token_and_wrong_key() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start(tmp.path(), "receiver", Visibility::Everyone);
    let s = Node::start(tmp.path(), "sender", Visibility::Everyone);

    let payload = r.core.begin_pairing_qr().unwrap();
    let mut qr = wooosh_core::pairing::QrPayload::parse(&payload).unwrap();
    qr.token[0] ^= 0xFF;
    let err = s.core.pair_with_qr(qr.encode()).unwrap_err();
    assert!(matches!(err, WoooshError::Pairing(_)), "got {err:?}");
    // Shells drive the sheet off events, so every failure must arrive as one.
    expect_pairing_failure(&s, "bad token");

    let payload = r.core.begin_pairing_qr().unwrap();
    let mut qr = wooosh_core::pairing::QrPayload::parse(&payload).unwrap();
    qr.pubkey[0] ^= 0xFF;
    let err = s.core.pair_with_qr(qr.encode()).unwrap_err();
    assert!(matches!(err, WoooshError::QrKeyMismatch), "got {err:?}");
    expect_pairing_failure(&s, "wrong key");

    let payload = r.core.begin_pairing_qr().unwrap();
    let mut qr = wooosh_core::pairing::QrPayload::parse(&payload).unwrap();
    qr.expires_unix = 1;
    let err = s.core.pair_with_qr(qr.encode()).unwrap_err();
    assert!(matches!(err, WoooshError::Pairing(_)), "got {err:?}");
    expect_pairing_failure(&s, "expired QR");

    let err = s.core.pair_with_qr("not-a-wooosh-qr".to_string()).unwrap_err();
    assert!(matches!(err, WoooshError::InvalidQrPayload(_)), "got {err:?}");
    expect_pairing_failure(&s, "garbage payload");
}

/// Clears the queue afterwards so the next phase cannot see a stale event.
fn expect_pairing_failure(node: &Node, what: &str) {
    let (success, message) = wait_for(
        &node.rx,
        Duration::from_secs(10),
        &format!("PairingResult after {what}"),
        |e| match e {
            CoreEvent::PairingResult { success, message, .. } => {
                Some((*success, message.clone()))
            }
            _ => None,
        },
    );
    assert!(!success, "{what}: expected PairingResult{{success:false}}, message={message:?}");
    node.drain();
}

/// The black holes are *bound* UDP sockets, so the OS sends no ICMP
/// port-unreachable and each one costs a full connect timeout if dialled serially.
#[test]
fn pair_via_qr_races_hints_so_a_dead_hint_first_does_not_stall() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start(tmp.path(), "receiver", Visibility::Everyone);
    let s = Node::start(tmp.path(), "sender", Visibility::Everyone);

    let blackhole_a = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let blackhole_b = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

    let payload = r.core.begin_pairing_qr().unwrap();
    let mut qr = wooosh_core::pairing::QrPayload::parse(&payload).unwrap();
    let live = std::mem::take(&mut qr.hints);
    qr.hints = vec![
        blackhole_a.local_addr().unwrap().to_string(),
        blackhole_b.local_addr().unwrap().to_string(),
    ];
    qr.hints.extend(live);
    assert_eq!(qr.hints.len(), 3);

    let t0 = Instant::now();
    let peer_id = s.core.pair_with_qr(qr.encode()).unwrap();
    let elapsed = t0.elapsed();
    eprintln!("[race] pair_with_qr with 2 dead hints ahead of the live one: {elapsed:?}");

    assert_eq!(peer_id, r.core.device_id().unwrap());
    // Serial dialling costs two connect timeouts before the live hint is even
    // tried; racing bounds it by the loopback handshake, so 5 s is generous.
    assert!(
        elapsed < Duration::from_secs(5),
        "hints look serial again: pairing took {elapsed:?}"
    );

    let ok = wait_for(&s.rx, Duration::from_secs(10), "sender PairingResult", |e| match e {
        CoreEvent::PairingResult { success, .. } => Some(*success),
        _ => None,
    });
    assert!(ok);
    let ok = wait_for(&r.rx, Duration::from_secs(10), "receiver PairingResult", |e| match e {
        CoreEvent::PairingResult { success, .. } => Some(*success),
        _ => None,
    });
    assert!(ok);

    // The losing dials must not leave a second connection behind.
    assert_eq!(r.core.trusted_peers().unwrap().len(), 1);
}

/// Bounded by one connect timeout, not the sum.
#[test]
fn pair_via_qr_all_hints_dead_fails_once_and_emits() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start(tmp.path(), "receiver", Visibility::Everyone);
    let s = Node::start(tmp.path(), "sender", Visibility::Everyone);

    let holes: Vec<std::net::UdpSocket> = (0..3)
        .map(|_| std::net::UdpSocket::bind("127.0.0.1:0").unwrap())
        .collect();

    let payload = r.core.begin_pairing_qr().unwrap();
    let mut qr = wooosh_core::pairing::QrPayload::parse(&payload).unwrap();
    qr.hints = holes.iter().map(|h| h.local_addr().unwrap().to_string()).collect();

    let t0 = Instant::now();
    let err = s.core.pair_with_qr(qr.encode()).unwrap_err();
    let elapsed = t0.elapsed();
    eprintln!("[race] pair_with_qr with 3 dead hints failed after: {elapsed:?}");

    assert!(matches!(err, WoooshError::Connect(_)), "got {err:?}");
    // Three raced hints share one 6 s deadline; serially they would be 18 s.
    assert!(
        elapsed < Duration::from_secs(12),
        "dead hints look serial again: {elapsed:?}"
    );
    expect_pairing_failure(&s, "all hints dead");
}

#[test]
fn pair_via_sas_and_mitm_codes_differ() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Node::start(tmp.path(), "alice", Visibility::Everyone);
    let m = Node::start(tmp.path(), "mallory", Visibility::Everyone);
    let b = Node::start(tmp.path(), "bob", Visibility::Everyone);

    let pid_m = a.core.connect_peer(m.addr(), None).unwrap();
    a.core.request_sas_pairing(pid_m.clone()).unwrap();
    let code_a = wait_for(&a.rx, Duration::from_secs(10), "alice PairingSas", |e| match e {
        CoreEvent::PairingSas { code, .. } => Some(code.clone()),
        _ => None,
    });
    let code_m1 = wait_for(&m.rx, Duration::from_secs(10), "mallory PairingSas", |e| match e {
        CoreEvent::PairingSas { code, .. } => Some(code.clone()),
        _ => None,
    });
    assert_eq!(code_a, code_m1);
    assert_eq!(code_a.len(), 6);
    assert!(code_a.chars().all(|c| c.is_ascii_digit()));

    // Session 2, the MITM's relayed one: the exporter binds each TLS transcript,
    // so the two sessions cannot show the same code (1e-6 collision floor).
    let pid_b = m.core.connect_peer(b.addr(), None).unwrap();
    m.core.request_sas_pairing(pid_b.clone()).unwrap();
    let code_m2 = wait_for(&m.rx, Duration::from_secs(10), "mallory PairingSas 2", |e| match e {
        CoreEvent::PairingSas { peer_id, code } if *peer_id == pid_b => Some(code.clone()),
        _ => None,
    });
    let code_b = wait_for(&b.rx, Duration::from_secs(10), "bob PairingSas", |e| match e {
        CoreEvent::PairingSas { code, .. } => Some(code.clone()),
        _ => None,
    });
    assert_eq!(code_m2, code_b);
    assert_ne!(
        code_a, code_m2,
        "relayed MITM sessions must derive different SAS codes"
    );

    let a_id = a.core.device_id().unwrap();
    a.core.confirm_sas(pid_m.clone(), true).unwrap();
    m.core.confirm_sas(a_id, true).unwrap();
    let ok = wait_for(&a.rx, Duration::from_secs(10), "alice PairingResult", |e| match e {
        CoreEvent::PairingResult { success, .. } => Some(*success),
        _ => None,
    });
    assert!(ok);
    let ok = wait_for(&m.rx, Duration::from_secs(10), "mallory PairingResult", |e| match e {
        CoreEvent::PairingResult { success, .. } => Some(*success),
        _ => None,
    });
    assert!(ok);
}

#[test]
fn transfer_200mib_and_500_small_files() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start(tmp.path(), "receiver", Visibility::Everyone);
    let s = Node::start(tmp.path(), "sender", Visibility::Everyone);

    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let big = src.join("big.bin");
    write_random_file(&big, 200 * MIB, 1);
    let mut paths = vec![big.to_string_lossy().to_string()];
    for i in 0..500 {
        let p = src.join(format!("small-{i:03}.bin"));
        write_random_file(&p, 100 * 1024, 1000 + i);
        paths.push(p.to_string_lossy().to_string());
    }

    // Unpaired accept-once transfer, PROTOCOL.md §4.4.
    let pid = s.core.connect_peer(r.addr(), None).unwrap();
    let tid = s.core.send(pid, paths).unwrap();

    let (offer_tid, fids, trusted, offer_key, offer_fp, offer_dt) = wait_for(
        &r.rx,
        Duration::from_secs(120),
        "IncomingOffer",
        |e| match e {
            CoreEvent::IncomingOffer {
                transfer_id,
                files,
                trusted,
                peer_pubkey,
                fingerprint,
                device_type,
                ..
            } => Some((
                transfer_id.clone(),
                files.iter().map(|f| f.fid).collect::<Vec<_>>(),
                *trusted,
                peer_pubkey.clone(),
                fingerprint.clone(),
                device_type.clone(),
            )),
            _ => None,
        },
    );
    assert_eq!(offer_tid, tid);
    assert_eq!(fids.len(), 501);
    assert!(!trusted, "unpaired sender must be untrusted");
    // The consent sheet identifies an unknown sender without deriving crypto.
    assert_eq!(offer_key, s.core.public_key().unwrap());
    assert_eq!(offer_fp, s.core.fingerprint_phrase().unwrap());
    assert!(matches!(offer_dt, Some(DeviceType::Desktop)));
    r.core.respond_to_offer(offer_tid, fids).unwrap();

    let n = wait_for(&s.rx, Duration::from_secs(60), "sender TransferStarted", |e| match e {
        CoreEvent::TransferStarted { direction: TransferDirection::Send, files, .. } => {
            Some(files.len())
        }
        _ => None,
    });
    assert_eq!(n, 501);
    let n = wait_for(&r.rx, Duration::from_secs(60), "receiver TransferStarted", |e| match e {
        CoreEvent::TransferStarted { direction: TransferDirection::Receive, files, .. } => {
            Some(files.len())
        }
        _ => None,
    });
    assert_eq!(n, 501);

    let (ok_r, fail_r, dur_r, bytes_r) = wait_for(
        &r.rx,
        Duration::from_secs(600),
        "receiver TransferDone",
        |e| match e {
            CoreEvent::TransferDone {
                ok_files,
                failed_files,
                duration_ms,
                bytes_transferred,
                ..
            } => Some((*ok_files, *failed_files, *duration_ms, *bytes_transferred)),
            _ => None,
        },
    );
    assert_eq!((ok_r, fail_r), (501, 0));
    let (ok_s, fail_s, dur_s) = wait_for(
        &s.rx,
        Duration::from_secs(120),
        "sender TransferDone",
        |e| match e {
            CoreEvent::TransferDone { ok_files, failed_files, duration_ms, .. } => {
                Some((*ok_files, *failed_files, *duration_ms))
            }
            _ => None,
        },
    );
    assert_eq!((ok_s, fail_s), (501, 0));
    assert!(dur_r > 0 && dur_s > 0, "duration_ms must be reported (r={dur_r}, s={dur_s})");
    println!(
        "transfer summary: {bytes_r} bytes in {dur_r} ms ({:.1} MB/s receiver-side)",
        bytes_r as f64 / 1e6 / (dur_r as f64 / 1000.0)
    );

    let files_dir = r.staging().join(&tid).join("files");
    let received: Vec<_> = std::fs::read_dir(&files_dir).unwrap().collect();
    assert_eq!(received.len(), 501);
    assert_eq!(b3_of(&files_dir.join("big.bin")), b3_of(&big));
    for i in [0usize, 123, 499] {
        let name = format!("small-{i:03}.bin");
        assert_eq!(b3_of(&files_dir.join(&name)), b3_of(&src.join(&name)), "{name}");
    }
}

#[test]
fn resume_after_receiver_restart_does_not_resend_verified_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let total = 200 * MIB;
    let r1 = Node::start(tmp.path(), "recv", Visibility::Everyone);
    let s = Node::start(tmp.path(), "send", Visibility::Everyone);

    // RESUME_Q requires a trusted channel, so pair first.
    let payload = r1.core.begin_pairing_qr().unwrap();
    let pid = s.core.pair_with_qr(payload).unwrap();
    let r_pubkey = r1.core.public_key().unwrap();

    let big = tmp.path().join("big.bin");
    write_random_file(&big, total, 7);
    let src_hash = b3_of(&big);

    let tid = s.core.send(pid, vec![big.to_string_lossy().to_string()]).unwrap();
    let (offer_tid, fids) = wait_for(&r1.rx, Duration::from_secs(60), "offer", |e| match e {
        CoreEvent::IncomingOffer { transfer_id, files, .. } => {
            Some((transfer_id.clone(), files.iter().map(|f| f.fid).collect::<Vec<_>>()))
        }
        _ => None,
    });
    r1.core.respond_to_offer(offer_tid, fids).unwrap();

    // >40 MiB received means at least two 16 MiB ledger fsyncs have landed, so
    // the abrupt kill below leaves persisted progress behind.
    wait_for(&r1.rx, Duration::from_secs(300), "mid-transfer progress", |e| match e {
        CoreEvent::Progress { bytes_done, .. } if *bytes_done >= 40 * MIB => Some(()),
        _ => None,
    });
    r1.core.stop();

    let resumable = wait_for(
        &s.rx,
        Duration::from_secs(120),
        "sender TransferError",
        |e| match e {
            CoreEvent::TransferError { transfer_id, resumable, .. } if *transfer_id == tid => {
                Some(*resumable)
            }
            _ => None,
        },
    );
    assert!(resumable);

    // Same state dir (identity, trust store, staging + ledger), new port.
    let r2 = Node::start(tmp.path(), "recv", Visibility::Everyone);
    let pid2 = s.core.connect_peer(r2.addr(), Some(r_pubkey)).unwrap();
    s.core.resume_transfer(pid2, tid.clone()).unwrap();

    let (ok_s, _) = wait_for(&s.rx, Duration::from_secs(600), "sender TransferDone", |e| {
        match e {
            CoreEvent::TransferDone { transfer_id, ok_files, failed_files, .. }
                if *transfer_id == tid =>
            {
                Some((*ok_files, *failed_files))
            }
            _ => None,
        }
    });
    assert_eq!(ok_s, 1);

    let stats = s.core.transfer_stats(&tid).expect("sender stats");
    assert!(stats.is_sender);
    assert!(
        stats.resumed_from >= 16 * MIB,
        "receiver should have resumed from a persisted ledger offset, got {}",
        stats.resumed_from
    );
    assert_eq!(
        stats.bytes_this_attempt,
        total - stats.resumed_from,
        "second attempt must send exactly the unverified remainder"
    );
    assert!(stats.bytes_this_attempt < total, "must not re-send the whole file");
    println!(
        "resume evidence: total={total} resumed_from={} second_attempt_bytes={}",
        stats.resumed_from, stats.bytes_this_attempt
    );

    wait_for(&r2.rx, Duration::from_secs(120), "receiver TransferDone", |e| match e {
        CoreEvent::TransferDone { ok_files: 1, failed_files: 0, .. } => Some(()),
        _ => None,
    });
    let received = r2.staging().join(&tid).join("files").join("big.bin");
    assert_eq!(b3_of(&received), src_hash);
}

#[test]
fn untrusted_channel_restrictions() {
    let tmp = tempfile::tempdir().unwrap();

    // PAIRING_REQUIRED must surface as a typed error, not a transport failure.
    let r = Node::start(tmp.path(), "paired-only", Visibility::PairedOnly);
    let s = Node::start(tmp.path(), "stranger", Visibility::Everyone);
    let err = s.core.connect_peer(r.addr(), None).unwrap_err();
    assert!(matches!(err, WoooshError::PairingRequired), "got {err:?}");
    let r_pid = r.core.device_id().unwrap();
    assert!(!s.core.peer_connected(&r_pid));

    // Under Everyone, OFFER is honored untrusted but RESUME_Q is not.
    let r2 = Node::start(tmp.path(), "everyone", Visibility::Everyone);
    let pid2 = s.core.connect_peer(r2.addr(), None).unwrap();
    assert!(s.core.peer_connected(&pid2));
    s.core.debug_send_control(&pid2, Msg::ResumeQ { tid: [9; 16] }).unwrap();
    wait_for(
        &s.rx,
        Duration::from_secs(10),
        "disconnect after untrusted RESUME_Q",
        |e| match e {
            CoreEvent::PeerDisconnected { peer_id } if *peer_id == pid2 => Some(()),
            _ => None,
        },
    );
    assert!(!s.core.peer_connected(&pid2));
}

/// SAS pairing never hands the shell a public key, so §4.5 pinning has to work
/// with the core re-deriving the pin from its own trust store.
#[test]
fn sas_paired_peer_is_pinned_on_reconnect_without_caller_key() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Node::start(tmp.path(), "alice", Visibility::Everyone);
    let b = Node::start(tmp.path(), "bob", Visibility::Everyone);
    let b_addr = b.addr();
    let b_pubkey = b.core.public_key().unwrap();
    let b_id = b.core.device_id().unwrap();

    let pid_b = a.core.connect_peer(b_addr.clone(), None).unwrap();
    a.core.request_sas_pairing(pid_b.clone()).unwrap();
    wait_for(&a.rx, Duration::from_secs(10), "alice PairingSas", |e| match e {
        CoreEvent::PairingSas { .. } => Some(()),
        _ => None,
    });
    wait_for(&b.rx, Duration::from_secs(10), "bob PairingSas", |e| match e {
        CoreEvent::PairingSas { .. } => Some(()),
        _ => None,
    });
    let a_id = a.core.device_id().unwrap();
    a.core.confirm_sas(pid_b.clone(), true).unwrap();
    b.core.confirm_sas(a_id, true).unwrap();
    // PairingResult carries the raw key, so a shell can pin or revoke later.
    let (paired_key, paired_fp) =
        wait_for(&a.rx, Duration::from_secs(10), "alice PairingResult", |e| match e {
            CoreEvent::PairingResult { success: true, peer_pubkey, fingerprint, .. } => {
                Some((peer_pubkey.clone(), fingerprint.clone()))
            }
            _ => None,
        });
    assert_eq!(paired_key, b_pubkey);
    assert_eq!(paired_fp, fingerprint_phrase_for(b_pubkey.clone()).unwrap());
    wait_for(&b.rx, Duration::from_secs(10), "bob PairingResult", |e| match e {
        CoreEvent::PairingResult { success: true, .. } => Some(()),
        _ => None,
    });

    a.drain();
    let pid2 = a.core.connect_peer(b_addr.clone(), None).unwrap();
    assert_eq!(pid2, b_id);
    let (ev_key, ev_trusted, ev_dt) = wait_for(
        &a.rx,
        Duration::from_secs(10),
        "alice PeerConnected (trusted)",
        |e| match e {
            CoreEvent::PeerConnected { peer_id, peer_pubkey, trusted, device_type, .. }
                if *peer_id == b_id =>
            {
                Some((peer_pubkey.clone(), *trusted, device_type.clone()))
            }
            _ => None,
        },
    );
    assert_eq!(ev_key, b_pubkey);
    assert!(ev_trusted, "a pinned peer must reconnect as trusted");
    assert!(matches!(ev_dt, Some(DeviceType::Desktop)), "HELLO dt must reach the shell");

    b.core.stop();
    drop(b);
    let imposter = Node::start_on(tmp.path(), "imposter", Visibility::Everyone, &b_addr);
    let imposter_key = imposter.core.public_key().unwrap();
    assert_ne!(imposter_key, b_pubkey);

    let err = a.core.connect_peer(b_addr.clone(), None).unwrap_err();
    assert!(matches!(err, WoooshError::KeyChanged), "got {err:?}");
    let (peer_id, expected, presented) =
        wait_for(&a.rx, Duration::from_secs(10), "alice KeyChanged", |e| match e {
            CoreEvent::KeyChanged { peer_id, expected_pubkey, presented_pubkey } => {
                Some((peer_id.clone(), expected_pubkey.clone(), presented_pubkey.clone()))
            }
            _ => None,
        });
    assert_eq!(peer_id, b_id);
    assert_eq!(expected, b_pubkey);
    assert_eq!(presented, Some(imposter_key), "the offending key must be reported");
    // No silent re-pin: bob's pin is untouched and the imposter is not stored.
    let pinned: Vec<Vec<u8>> =
        a.core.trusted_peers().unwrap().into_iter().map(|p| p.pubkey).collect();
    assert_eq!(pinned, vec![b_pubkey]);
}

#[test]
fn trusted_peers_reflects_pair_and_revoke() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start(tmp.path(), "receiver", Visibility::Everyone);
    let s = Node::start(tmp.path(), "sender", Visibility::Everyone);
    let r_pubkey = r.core.public_key().unwrap();
    let r_id = r.core.device_id().unwrap();

    assert!(s.core.trusted_peers().unwrap().is_empty());

    let payload = r.core.begin_pairing_qr().unwrap();
    s.core.pair_with_qr(payload).unwrap();
    wait_for(&s.rx, Duration::from_secs(10), "sender PairingResult", |e| match e {
        CoreEvent::PairingResult { success: true, .. } => Some(()),
        _ => None,
    });
    wait_for(&r.rx, Duration::from_secs(10), "receiver PairingResult", |e| match e {
        CoreEvent::PairingResult { success: true, .. } => Some(()),
        _ => None,
    });

    let list = s.core.trusted_peers().unwrap();
    assert_eq!(list.len(), 1);
    let p = &list[0];
    assert_eq!(p.pubkey, r_pubkey);
    assert_eq!(p.device_id, r_id);
    assert_eq!(p.device_name, "receiver");
    assert!(matches!(p.device_type, Some(DeviceType::Desktop)));
    assert_eq!(p.fingerprint, r.core.fingerprint_phrase().unwrap());
    assert_eq!(p.fingerprint, fingerprint_phrase_for(r_pubkey.clone()).unwrap());
    assert!(p.paired_at > 0 && p.last_seen >= p.paired_at);

    let r_list = r.core.trusted_peers().unwrap();
    assert_eq!(r_list.len(), 1);
    assert_eq!(r_list[0].pubkey, s.core.public_key().unwrap());
    assert_eq!(r_list[0].device_name, "sender");

    assert!(s.core.revoke_peer(p.pubkey.clone()).unwrap());
    assert!(s.core.trusted_peers().unwrap().is_empty());
    assert!(!s.core.revoke_peer(r_pubkey.clone()).unwrap(), "second revoke is a no-op");

    // Revoking drops the pin memory too: untrusted again, not KEY_CHANGED.
    s.drain();
    let pid = s.core.connect_peer(r.addr(), None).unwrap();
    let trusted = wait_for(
        &s.rx,
        Duration::from_secs(10),
        "sender PeerConnected after revoke",
        |e| match e {
            CoreEvent::PeerConnected { peer_id, trusted, .. } if *peer_id == pid => Some(*trusted),
            _ => None,
        },
    );
    assert!(!trusted, "revoked peer must come back untrusted");
}

#[test]
fn key_changed_on_pin_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let victim = Node::start(tmp.path(), "victim", Visibility::Everyone);
    let imposter = Node::start(tmp.path(), "imposter", Visibility::Everyone);
    let s = Node::start(tmp.path(), "sender", Visibility::Everyone);

    let victim_key = victim.core.public_key().unwrap();
    let err = s.core.connect_peer(imposter.addr(), Some(victim_key)).unwrap_err();
    assert!(matches!(err, WoooshError::KeyChanged), "got {err:?}");
    wait_for(&s.rx, Duration::from_secs(10), "KeyChanged event", |e| match e {
        CoreEvent::KeyChanged { .. } => Some(()),
        _ => None,
    });

    let ok_pid = s.core.connect_peer(victim.addr(), Some(victim.core.public_key().unwrap()));
    assert!(ok_pid.is_ok());
}

/// PROTOCOL.md §9, redeemer-sends direction; the shells ship the opposite one,
/// covered below. Relays off, so the ticket's direct candidates are the only way in.
#[test]
fn internet_ticket_transfer_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start_offline_internet(tmp.path(), "net-receiver", Visibility::Everyone);
    let s = Node::start_offline_internet(tmp.path(), "net-sender", Visibility::Everyone);

    let ticket = r.core.begin_internet_ticket().unwrap();
    assert!(ticket.starts_with("wooosh-net:1?nid="), "got {ticket}");

    let info = wooosh_core::parse_internet_ticket(ticket.clone()).unwrap();
    assert_eq!(info.device_name.as_deref(), Some("net-receiver"));
    assert_eq!(info.device_id, r.core.device_id().unwrap());
    assert_eq!(info.node_id, r.core.public_key().unwrap());
    assert!(!info.expired);

    let peer_id = s.core.redeem_ticket(ticket).unwrap();
    // A device must not appear as two identities depending on the path.
    assert_eq!(peer_id, r.core.device_id().unwrap());
    assert_eq!(
        wooosh_core::device_id_for(r.core.public_key().unwrap()).unwrap(),
        peer_id
    );
    assert_eq!(
        wooosh_core::fingerprint_phrase_for(r.core.public_key().unwrap()).unwrap(),
        fingerprint_phrase_for(r.core.public_key().unwrap()).unwrap()
    );

    // Redeeming authorises one session and pairs nothing (PROTOCOL.md §9.4).
    let redeemer = wait_for(&s.rx, Duration::from_secs(20), "sender TicketRedeemed", |e| match e {
        CoreEvent::TicketRedeemed { peer_id, .. } => Some(peer_id.clone()),
        _ => None,
    });
    assert_eq!(redeemer, r.core.device_id().unwrap());
    let publisher = wait_for(&r.rx, Duration::from_secs(20), "receiver TicketRedeemed", |e| match e {
        CoreEvent::TicketRedeemed { peer_id, .. } => Some(peer_id.clone()),
        _ => None,
    });
    assert_eq!(publisher, s.core.device_id().unwrap());
    assert!(s.core.trusted_peers().unwrap().is_empty(), "redeeming wrote a pin");
    assert!(r.core.trusted_peers().unwrap().is_empty(), "publishing wrote a pin");

    let src = tmp.path().join("net-payload.bin");
    write_random_file(&src, 3 * MIB, 0xBEEF);
    let want = b3_of(&src);

    let tid = s
        .core
        .send(peer_id, vec![src.to_string_lossy().to_string()])
        .unwrap();

    let (rtid, fids) = wait_for(&r.rx, Duration::from_secs(30), "IncomingOffer", |e| match e {
        CoreEvent::IncomingOffer { transfer_id, files, .. } => {
            Some((transfer_id.clone(), files.iter().map(|f| f.fid).collect::<Vec<_>>()))
        }
        _ => None,
    });
    assert_eq!(rtid, tid);
    r.core.respond_to_offer(rtid.clone(), fids).unwrap();

    let staged = wait_for(&r.rx, Duration::from_secs(60), "FileReady", |e| match e {
        CoreEvent::FileReady { staged_path, .. } => Some(staged_path.clone()),
        _ => None,
    });
    assert_eq!(b3_of(Path::new(&staged)), want);

    let (ok_files, failed, bytes) =
        wait_for(&r.rx, Duration::from_secs(30), "TransferDone", |e| match e {
            CoreEvent::TransferDone { ok_files, failed_files, bytes_transferred, .. } => {
                Some((*ok_files, *failed_files, *bytes_transferred))
            }
            _ => None,
        });
    assert_eq!((ok_files, failed), (1, 0));
    assert_eq!(bytes, 3 * MIB);

    let dir = wait_for(&s.rx, Duration::from_secs(30), "TransferStarted", |e| match e {
        CoreEvent::TransferStarted { direction, .. } => Some(matches!(direction, TransferDirection::Send)),
        _ => None,
    });
    assert!(dir);
}

/// `PairedOnly` is the shipped default, and the internet path never pairs, so a
/// visibility check reading `trusted` alone kills an already-burned ticket.
#[test]
fn internet_ticket_transfer_works_under_paired_only() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start_offline_internet(tmp.path(), "po-receiver", Visibility::PairedOnly);
    let s = Node::start_offline_internet(tmp.path(), "po-sender", Visibility::PairedOnly);

    let ticket = r.core.begin_internet_ticket().unwrap();
    let peer_id = s.core.redeem_ticket(ticket).unwrap();

    wait_for(&r.rx, Duration::from_secs(20), "receiver TicketRedeemed", |e| match e {
        CoreEvent::TicketRedeemed { peer_id, .. } => Some(peer_id.clone()),
        _ => None,
    });

    let src = tmp.path().join("po-payload.bin");
    write_random_file(&src, 1 * MIB, 0x5A5A);
    let want = b3_of(&src);

    let tid = s
        .core
        .send(peer_id, vec![src.to_string_lossy().to_string()])
        .unwrap();

    // The regression: PairedOnly used to close here, so the OFFER never landed.
    let (rtid, fids) = wait_for(&r.rx, Duration::from_secs(30), "IncomingOffer", |e| match e {
        CoreEvent::IncomingOffer { transfer_id, files, .. } => {
            Some((transfer_id.clone(), files.iter().map(|f| f.fid).collect::<Vec<_>>()))
        }
        _ => None,
    });
    assert_eq!(rtid, tid);
    r.core.respond_to_offer(rtid, fids).unwrap();

    let staged = wait_for(&r.rx, Duration::from_secs(60), "FileReady", |e| match e {
        CoreEvent::FileReady { staged_path, .. } => Some(staged_path.clone()),
        _ => None,
    });
    assert_eq!(b3_of(Path::new(&staged)), want);

    assert!(s.core.trusted_peers().unwrap().is_empty());
    assert!(r.core.trusted_peers().unwrap().is_empty());
}

/// PROTOCOL.md §9.2 makes the publisher the receiver, but the shells ship the
/// reverse, so the engine must permit either direction.
#[test]
fn internet_publisher_can_send_to_the_redeemer_under_paired_only() {
    let tmp = tempfile::tempdir().unwrap();
    let pubr = Node::start_offline_internet(tmp.path(), "pub-sender", Visibility::PairedOnly);
    let red = Node::start_offline_internet(tmp.path(), "red-receiver", Visibility::PairedOnly);

    let ticket = pubr.core.begin_internet_ticket().unwrap();
    let publisher_id = red.core.redeem_ticket(ticket).unwrap();
    assert_eq!(publisher_id, pubr.core.device_id().unwrap());

    let redeemer_id =
        wait_for(&pubr.rx, Duration::from_secs(20), "publisher TicketRedeemed", |e| match e {
            CoreEvent::TicketRedeemed { peer_id, .. } => Some(peer_id.clone()),
            _ => None,
        });
    assert_eq!(redeemer_id, red.core.device_id().unwrap());

    let src = tmp.path().join("pub-payload.bin");
    write_random_file(&src, 1 * MIB, 0x3C3C);
    let want = b3_of(&src);

    let tid = pubr
        .core
        .send(redeemer_id, vec![src.to_string_lossy().to_string()])
        .unwrap();

    let (rtid, fids) = wait_for(&red.rx, Duration::from_secs(30), "IncomingOffer", |e| match e {
        CoreEvent::IncomingOffer { transfer_id, files, .. } => {
            Some((transfer_id.clone(), files.iter().map(|f| f.fid).collect::<Vec<_>>()))
        }
        _ => None,
    });
    assert_eq!(rtid, tid);
    red.core.respond_to_offer(rtid, fids).unwrap();

    let staged = wait_for(&red.rx, Duration::from_secs(60), "FileReady", |e| match e {
        CoreEvent::FileReady { staged_path, .. } => Some(staged_path.clone()),
        _ => None,
    });
    assert_eq!(b3_of(Path::new(&staged)), want);

    assert!(pubr.core.trusted_peers().unwrap().is_empty());
    assert!(red.core.trusted_peers().unwrap().is_empty());
}

/// A ticket is a capability: single-use, and dead once the publisher ends it.
#[test]
fn internet_ticket_is_single_use_and_revocable() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start_offline_internet(tmp.path(), "cap-receiver", Visibility::Everyone);
    let s1 = Node::start_offline_internet(tmp.path(), "cap-sender-1", Visibility::Everyone);
    let s2 = Node::start_offline_internet(tmp.path(), "cap-sender-2", Visibility::Everyone);

    let ticket = r.core.begin_internet_ticket().unwrap();
    assert_eq!(s1.core.redeem_ticket(ticket.clone()).unwrap(), r.core.device_id().unwrap());

    let err = s2.core.redeem_ticket(ticket).unwrap_err();
    assert!(matches!(err, WoooshError::Pairing(_)), "got {err:?}");
    expect_pairing_failure(&s2, "reused ticket");
    assert!(!r
        .core
        .trusted_peers()
        .unwrap()
        .iter()
        .any(|p| p.pubkey == s2.core.public_key().unwrap()));

    let ticket2 = r.core.begin_internet_ticket().unwrap();
    r.core.end_internet_ticket().unwrap();
    let err = s2.core.redeem_ticket(ticket2).unwrap_err();
    assert!(matches!(err, WoooshError::Pairing(_)), "got {err:?}");
    expect_pairing_failure(&s2, "withdrawn ticket");
}

/// A ticket advertises whatever relay is configured at runtime (DESIGN.md §9.1),
/// which is how a redeemer ends up on the publisher's relay.
#[test]
fn relay_urls_are_settable_and_validated() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start_offline_internet(tmp.path(), "relay-cfg", Visibility::Everyone);

    let ticket = NetTicketFields::of(&r.core.begin_internet_ticket().unwrap());
    assert!(ticket.relay.is_none(), "relay-free node advertised {:?}", ticket.relay);
    assert!(!ticket.addrs.is_empty(), "relay-free node advertised no address to dial");

    // A typo in Settings must not take the internet path down.
    let err = r
        .core
        .set_relay_urls(Some(vec!["not a url".into()]))
        .unwrap_err();
    assert!(matches!(err, WoooshError::InvalidArgument(_)), "got {err:?}");
    let after = NetTicketFields::of(&r.core.begin_internet_ticket().unwrap());
    assert!(after.relay.is_none(), "rejected URL still changed the relay");

    // Failing at publish time is the point: a ticket that silently fell back to
    // local addresses would only work on the publisher's own network.
    r.core
        .set_relay_urls(Some(vec!["https://relay.invalid./".into()]))
        .unwrap();
    let err = r.core.begin_internet_ticket().unwrap_err();
    assert!(matches!(err, WoooshError::Connect(_)), "got {err:?}");

    r.core.set_relay_urls(Some(Vec::new())).unwrap();
    let recovered = NetTicketFields::of(&r.core.begin_internet_ticket().unwrap());
    assert!(!recovered.addrs.is_empty(), "node did not recover after a bad relay setting");

    // Tickets minted before a relay change are dead by construction (the change
    // tears the endpoint down). Not asserted: dialling one costs 30 s.
}

/// The shells' parse helper deliberately does not expose `addrs`.
struct NetTicketFields {
    relay: Option<String>,
    addrs: Vec<String>,
}

impl NetTicketFields {
    fn of(ticket: &str) -> Self {
        let params = ticket.split_once('?').expect("ticket has params").1;
        let mut relay = None;
        let mut addrs = Vec::new();
        for kv in params.split('&') {
            match kv.split_once('=') {
                Some(("relay", v)) => relay = Some(percent_decode(v)),
                Some(("addrs", v)) => {
                    addrs = v.split(',').filter(|a| !a.is_empty()).map(String::from).collect()
                }
                _ => {}
            }
        }
        Self { relay, addrs }
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// Every outcome still reaches the host as a `PairingResult` (DESIGN.md §4).
#[test]
fn internet_ticket_rejects_bad_payloads() {
    let tmp = tempfile::tempdir().unwrap();
    let s = Node::start_offline_internet(tmp.path(), "bad-ticket-sender", Visibility::Everyone);

    let err = s.core.redeem_ticket("not-a-ticket".into()).unwrap_err();
    assert!(matches!(err, WoooshError::InvalidQrPayload(_)), "got {err:?}");
    expect_pairing_failure(&s, "garbage ticket");

    let err = s
        .core
        .redeem_ticket(s.core.begin_pairing_qr().unwrap())
        .unwrap_err();
    assert!(matches!(err, WoooshError::InvalidQrPayload(_)), "got {err:?}");
    expect_pairing_failure(&s, "QR payload as ticket");

    let expired = wooosh_core::inet::NetTicket {
        version: 1,
        node_id: [7u8; 32],
        token: [8u8; 32],
        dn: None,
        relay: None,
        direct: vec![],
        expires_unix: 1,
    };
    let err = s.core.redeem_ticket(expired.encode()).unwrap_err();
    assert!(matches!(err, WoooshError::Pairing(_)), "got {err:?}");
    expect_pairing_failure(&s, "expired ticket");
}

/// PROTOCOL.md §4.1: a property of the channel, not of the transport.
#[test]
fn internet_paired_only_honours_a_live_ticket_and_rejects_without_one() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start_offline_internet(tmp.path(), "strict-receiver", Visibility::Everyone);
    let ticket = r.core.begin_internet_ticket().unwrap();
    r.core.set_visibility(Visibility::PairedOnly).unwrap();

    // A live ticket is an invitation: PairedOnly must not close before the
    // caller can present its token.
    let s = Node::start_offline_internet(tmp.path(), "strict-sender", Visibility::Everyone);
    assert_eq!(s.core.redeem_ticket(ticket.clone()).unwrap(), r.core.device_id().unwrap());
    assert!(r.core.trusted_peers().unwrap().is_empty());

    let s2 = Node::start_offline_internet(tmp.path(), "strict-sender-2", Visibility::Everyone);
    let err = s2.core.redeem_ticket(ticket).unwrap_err();
    assert!(
        matches!(err, WoooshError::PairingRequired | WoooshError::Pairing(_)),
        "got {err:?}"
    );
}

/// PROTOCOL.md §4.3 + §9.5: iroh is TLS 1.3 too, so SAS works unchanged off-LAN.
#[test]
fn internet_sas_codes_agree() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start_offline_internet(tmp.path(), "sas-net-receiver", Visibility::Everyone);
    let s = Node::start_offline_internet(tmp.path(), "sas-net-sender", Visibility::Everyone);

    let ticket = r.core.begin_internet_ticket().unwrap();
    let peer_id = s.core.redeem_ticket(ticket).unwrap();
    let s_id = s.core.device_id().unwrap();

    s.core.request_sas_pairing(peer_id.clone()).unwrap();
    let a = wait_for(&s.rx, Duration::from_secs(10), "sender SAS", |e| match e {
        CoreEvent::PairingSas { code, .. } => Some(code.clone()),
        _ => None,
    });
    let b = wait_for(&r.rx, Duration::from_secs(10), "receiver SAS", |e| match e {
        CoreEvent::PairingSas { peer_id, code } if *peer_id == s_id => Some(code.clone()),
        _ => None,
    });
    assert_eq!(a, b, "SAS must agree across an iroh session");
    assert_eq!(a.len(), 6);
}

/// Needs real network access, so it is not part of CI. Run explicitly with:
///   cargo test --release --test integration -- --ignored --test-threads=1 internet_ticket_via_public_relays
#[test]
#[ignore = "requires internet access and n0's public relay infrastructure"]
fn internet_ticket_via_public_relays() {
    let tmp = tempfile::tempdir().unwrap();
    let r = Node::start(tmp.path(), "relay-receiver", Visibility::Everyone);
    let s = Node::start(tmp.path(), "relay-sender", Visibility::Everyone);

    let ticket = r.core.begin_internet_ticket().unwrap();
    let info = wooosh_core::parse_internet_ticket(ticket.clone()).unwrap();
    assert!(info.relay.is_some(), "a default-configured ticket must carry a home relay");

    let peer_id = s.core.redeem_ticket(ticket).unwrap();
    assert_eq!(peer_id, r.core.device_id().unwrap());

    let src = tmp.path().join("relay-payload.bin");
    write_random_file(&src, MIB, 0xF00D);
    let want = b3_of(&src);
    let tid = s
        .core
        .send(peer_id, vec![src.to_string_lossy().to_string()])
        .unwrap();
    let (rtid, fids) = wait_for(&r.rx, Duration::from_secs(60), "IncomingOffer", |e| match e {
        CoreEvent::IncomingOffer { transfer_id, files, .. } => {
            Some((transfer_id.clone(), files.iter().map(|f| f.fid).collect::<Vec<_>>()))
        }
        _ => None,
    });
    assert_eq!(rtid, tid);
    r.core.respond_to_offer(rtid, fids).unwrap();
    let staged = wait_for(&r.rx, Duration::from_secs(120), "FileReady", |e| match e {
        CoreEvent::FileReady { staged_path, .. } => Some(staged_path.clone()),
        _ => None,
    });
    assert_eq!(b3_of(Path::new(&staged)), want);
}

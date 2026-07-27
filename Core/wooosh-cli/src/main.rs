//! wooosh-cli — thin CLI over wooosh-core for testing and interop.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use wooosh_core::{
    Config, CoreEvent, CoreEventListener, DeviceType, FileKeyStore, KeyStore, Visibility,
    WoooshCore,
};

#[derive(Parser)]
#[command(name = "wooosh-cli", about = "Wooosh core test CLI", version)]
struct Cli {
    /// State directory (identity key, trust store). Default: ~/.wooosh-cli
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    /// Device name announced in HELLO.
    #[arg(long, global = true, default_value = "wooosh-cli")]
    name: String,
    /// Internet-path relay URL; repeat for several. Default: n0's public relays.
    #[arg(long, global = true)]
    relay: Vec<String>,
    /// Internet path without any relay or address lookup: direct connections
    /// only, from the addresses carried in the ticket.
    #[arg(long, global = true)]
    no_relay: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Listen for offers; auto-accept and move finished files to --out.
    Receive {
        /// Staging directory for in-flight transfers.
        #[arg(long)]
        staging: PathBuf,
        /// Where verified files are moved (default: current directory).
        #[arg(long, default_value = ".")]
        out: PathBuf,
        /// UDP listen address.
        #[arg(long, default_value = "0.0.0.0:0")]
        listen: String,
        /// everyone | paired-only | off
        #[arg(long, default_value = "everyone")]
        visibility: String,
        /// Print a QR pairing payload on startup.
        #[arg(long)]
        qr: bool,
        /// Exit after one completed transfer (for scripting).
        #[arg(long)]
        once: bool,
    },
    /// Send files to a receiver at addr (ip:port).
    Send {
        addr: String,
        files: Vec<PathBuf>,
        /// Staging directory (unused for sending, but required by config).
        #[arg(long)]
        staging: Option<PathBuf>,
        /// Expected peer public key, hex (pins the connection).
        #[arg(long)]
        expect_key: Option<String>,
        /// Pair via a QR payload string before sending.
        #[arg(long)]
        qr: Option<String>,
    },
    /// Internet path (PROTOCOL.md §9), receiver side: publish an iroh ticket
    /// and wait. Print the ticket, hand it to the sender over any channel.
    InternetTicket {
        /// Staging directory for in-flight transfers.
        #[arg(long)]
        staging: PathBuf,
        /// Where verified files are moved (default: current directory).
        #[arg(long, default_value = ".")]
        out: PathBuf,
        /// Exit after one completed transfer (for scripting).
        #[arg(long)]
        once: bool,
    },
    /// Internet path, sender side: redeem a ticket and send files over it.
    InternetRedeem {
        ticket: String,
        files: Vec<PathBuf>,
        /// Staging directory (unused for sending, but required by config).
        #[arg(long)]
        staging: Option<PathBuf>,
    },
    /// Print this device's identity.
    Id,
    /// List pinned peers from the trust store; optionally revoke one.
    Trust {
        /// Revoke this peer public key (hex) instead of only listing.
        #[arg(long)]
        revoke: Option<String>,
    },
}

struct Listener {
    tx: mpsc::Sender<CoreEvent>,
}

impl CoreEventListener for Listener {
    fn on_event(&self, event: CoreEvent) {
        let _ = self.tx.send(event);
    }
}

fn start_core(
    state_dir: &PathBuf,
    name: &str,
    staging: PathBuf,
    listen: Option<String>,
    visibility: Visibility,
    relay_urls: Option<Vec<String>>,
) -> Result<(Arc<WoooshCore>, mpsc::Receiver<CoreEvent>)> {
    std::fs::create_dir_all(state_dir)?;
    let core = WoooshCore::new();
    let (tx, rx) = mpsc::channel();
    let key_store: Arc<dyn KeyStore> = FileKeyStore::new(
        state_dir.join("identity.key").to_string_lossy().to_string(),
    );
    core.start(
        Config {
            device_name: name.to_string(),
            device_type: DeviceType::Desktop,
            visibility,
            staging_dir: staging.to_string_lossy().to_string(),
            trust_store_path: state_dir.join("trust.json").to_string_lossy().to_string(),
            listen_addr: listen,
            relay_urls,
        },
        key_store,
        Arc::new(Listener { tx }),
    )?;
    Ok((core, rx))
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    let state_dir = cli.state_dir.clone().unwrap_or_else(|| {
        dirs_fallback().join(".wooosh-cli")
    });
    // None = n0's public relays; Some(vec![]) = no relays at all.
    let relays: Option<Vec<String>> = if cli.no_relay {
        Some(Vec::new())
    } else if cli.relay.is_empty() {
        None
    } else {
        Some(cli.relay.clone())
    };

    match cli.cmd {
        Cmd::Id => {
            let staging = std::env::temp_dir().join("wooosh-cli-id-staging");
            let (core, _rx) = start_core(&state_dir, &cli.name, staging, None, Visibility::Off, relays.clone())?;
            println!("device_id:   {}", core.device_id()?);
            println!("fingerprint: {}", core.fingerprint_phrase()?);
            println!("pubkey_hex:  {}", hex(&core.public_key()?));
            core.stop();
        }
        Cmd::Trust { revoke } => {
            let staging = std::env::temp_dir().join("wooosh-cli-trust-staging");
            let (core, _rx) = start_core(&state_dir, &cli.name, staging, None, Visibility::Off, relays.clone())?;
            if let Some(h) = revoke {
                let removed = core.revoke_peer(unhex(&h)?)?;
                println!("revoked: {removed}");
            }
            for p in core.trusted_peers()? {
                println!(
                    "{}  {}  dt={:?}  pubkey={}  paired_at={}  last_seen={}\n    {}",
                    p.device_id,
                    p.device_name,
                    p.device_type,
                    hex(&p.pubkey),
                    p.paired_at,
                    p.last_seen,
                    p.fingerprint
                );
            }
            core.stop();
        }
        Cmd::InternetTicket { staging, out, once } => {
            std::fs::create_dir_all(&out)?;
            let (core, rx) =
                start_core(&state_dir, &cli.name, staging, None, Visibility::Everyone, relays.clone())?;
            println!("device_id:   {}", core.device_id()?);
            println!("pubkey_hex:  {}", hex(&core.public_key()?));
            println!("publishing on iroh (this contacts a public relay)...");
            let ticket = core.begin_internet_ticket()?;
            println!("ticket:      {ticket}");
            println!("redeem with: wooosh-cli internet-redeem '{ticket}' <files...>");
            receive_loop(&core, &rx, &out, once)?;
            return Ok(());
        }
        Cmd::InternetRedeem { ticket, files, staging } => {
            if files.is_empty() {
                bail!("no files to send");
            }
            for f in &files {
                if !f.exists() {
                    bail!("no such file: {}", f.display());
                }
            }
            let staging =
                staging.unwrap_or_else(|| std::env::temp_dir().join("wooosh-cli-send-staging"));
            let (core, rx) =
                start_core(&state_dir, &cli.name, staging, None, Visibility::Everyone, relays.clone())?;
            let info = wooosh_core::parse_internet_ticket(ticket.clone())?;
            println!(
                "ticket for {} ({}), relay={:?}, expired={}",
                info.device_id,
                info.device_name.as_deref().unwrap_or("?"),
                info.relay,
                info.expired
            );
            let peer_id = core.redeem_ticket(ticket)?;
            println!("connected to {peer_id} over the internet");
            let tid = core.send(
                peer_id,
                files.iter().map(|f| f.to_string_lossy().to_string()).collect(),
            )?;
            println!("transfer {tid} offered");
            return send_loop(&core, &rx);
        }
        Cmd::Receive { staging, out, listen, visibility, qr, once } => {
            let vis = match visibility.as_str() {
                "everyone" => Visibility::Everyone,
                "paired-only" => Visibility::PairedOnly,
                "off" => Visibility::Off,
                v => bail!("bad visibility {v}"),
            };
            std::fs::create_dir_all(&out)?;
            let (core, rx) = start_core(&state_dir, &cli.name, staging, Some(listen), vis, relays.clone())?;
            println!("listening on {}", core.listen_addr()?);
            println!("device_id:   {}", core.device_id()?);
            println!("pubkey_hex:  {}", hex(&core.public_key()?));
            if qr {
                println!("qr_payload:  {}", core.begin_pairing_qr()?);
            }
            receive_loop(&core, &rx, &out, once)?;
            return Ok(());
        }
        Cmd::Send { addr, files, staging, expect_key, qr } => {
            if files.is_empty() {
                bail!("no files to send");
            }
            for f in &files {
                if !f.exists() {
                    bail!("no such file: {}", f.display());
                }
            }
            let staging = staging
                .unwrap_or_else(|| std::env::temp_dir().join("wooosh-cli-send-staging"));
            let (core, rx) =
                start_core(&state_dir, &cli.name, staging, None, Visibility::Everyone, relays.clone())?;

            let peer_id = if let Some(payload) = qr {
                let pid = core.pair_with_qr(payload)?;
                println!("paired with {pid}");
                pid
            } else {
                let expected = expect_key.map(|h| unhex(&h)).transpose()?;
                core.connect_peer(addr, expected)?
            };
            println!("connected to {peer_id}");

            let tid = core.send(
                peer_id,
                files.iter().map(|f| f.to_string_lossy().to_string()).collect(),
            )?;
            println!("transfer {tid} offered");
            return send_loop(&core, &rx);
        }
    }
    Ok(())
}

/// Shared by the LAN and internet paths: a host drives both with the same
/// events (DESIGN.md §9.1).
fn receive_loop(
    core: &Arc<WoooshCore>,
    rx: &mpsc::Receiver<CoreEvent>,
    out: &PathBuf,
    once: bool,
) -> Result<()> {
    loop {
        match rx.recv().context("event channel closed")? {
            CoreEvent::IncomingOffer { transfer_id, from_name, trusted, files, total_bytes, .. } => {
                println!(
                    "offer {transfer_id} from {from_name} (trusted={trusted}): {} files, {total_bytes} bytes — auto-accepting",
                    files.len()
                );
                let fids = files.iter().map(|f| f.fid).collect();
                core.respond_to_offer(transfer_id, fids)?;
            }
            CoreEvent::FileReady { file_id, staged_path, .. } => {
                // Storage routing is the native shell's job; stand in for it.
                let src = PathBuf::from(&staged_path);
                let dest = out.join(src.file_name().unwrap());
                std::fs::rename(&src, &dest)
                    .or_else(|_| std::fs::copy(&src, &dest).map(|_| ()))?;
                println!("file {file_id} ready -> {}", dest.display());
            }
            CoreEvent::TransferDone {
                transfer_id,
                ok_files,
                failed_files,
                bytes_transferred,
                duration_ms,
            } => {
                println!(
                    "transfer {transfer_id} done: {ok_files} ok, {failed_files} failed, {bytes_transferred} bytes in {duration_ms} ms"
                );
                if once {
                    core.stop();
                    return Ok(());
                }
            }
            CoreEvent::PairingSas { peer_id, code } => {
                println!("SAS code for {peer_id}: {code} — auto-confirming");
                core.confirm_sas(peer_id, true)?;
            }
            CoreEvent::TransferError { transfer_id, error, resumable } => {
                println!("transfer {transfer_id} error: {error} (resumable={resumable})");
            }
            other => println!("{other:?}"),
        }
    }
}

fn send_loop(core: &Arc<WoooshCore>, rx: &mpsc::Receiver<CoreEvent>) -> Result<()> {
    loop {
        match rx.recv_timeout(Duration::from_secs(300)) {
            Ok(CoreEvent::Progress { bytes_done, total_bytes, rate_bps, .. }) => {
                println!(
                    "progress {bytes_done}/{total_bytes} ({:.1} MB/s)",
                    rate_bps as f64 / 1e6
                );
            }
            Ok(CoreEvent::TransferDone {
                ok_files,
                failed_files,
                bytes_transferred,
                duration_ms,
                ..
            }) => {
                println!(
                    "done: {ok_files} ok, {failed_files} failed, {bytes_transferred} bytes sent in {duration_ms} ms"
                );
                core.stop();
                if failed_files > 0 {
                    std::process::exit(1);
                }
                return Ok(());
            }
            Ok(CoreEvent::TransferError { error, resumable, .. }) => {
                core.stop();
                bail!("transfer failed: {error} (resumable={resumable})");
            }
            Ok(other) => println!("{other:?}"),
            Err(_) => {
                core.stop();
                bail!("timed out waiting for transfer completion");
            }
        }
    }
}

fn dirs_fallback() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        bail!("odd hex length");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).context("bad hex"))
        .collect()
}

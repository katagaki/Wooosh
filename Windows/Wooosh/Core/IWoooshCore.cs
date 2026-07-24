namespace Wooosh.Core;

/// <summary>
/// Shell-side seam over the wooosh-core FFI surface (DESIGN.md §4).
///
/// The contract is deliberately coarse: commands in, an event stream out. The shell never
/// sees a socket or a key.
///
/// <para><b>Threading (normative, DESIGN.md §4).</b> Every exported core call is
/// synchronous and blocking, and none of them may run on the UI thread. The blocking ones
/// are exposed as Task-returning members here and are expected to be awaited off a
/// background thread; the cheap ones (visibility, respond-to-offer, cancel) are plain
/// synchronous calls. <see cref="StartAsync"/> in particular blocks on the platform key store,
/// which can take arbitrarily long behind a Windows Hello prompt.</para>
///
/// <para><b>Implementations.</b> <see cref="NativeWoooshCore"/> is the only one, and it
/// talks to <c>wooosh_core.dll</c>. There is deliberately no mock or fake implementation:
/// a shell that can invent peers and transfers is a shell whose screenshots and demos stop
/// meaning anything. If the native core is missing, the app must say so, not pretend.</para>
/// </summary>
public interface IWoooshCore : IDisposable
{
    /// <summary>
    /// Core to shell event stream. Raised on the core's event thread; handlers must
    /// marshal to the UI thread themselves.
    /// </summary>
    event Action<CoreEvent>? EventReceived;

    /// <summary>Blocking. Boots the engine, loads the identity key, binds the QUIC socket.</summary>
    Task StartAsync(CoreConfig config, CancellationToken cancellationToken = default);

    /// <summary>Blocking (roughly 2 s of runtime shutdown plus joining the event thread).</summary>
    Task StopAsync();

    void SetVisibility(CoreVisibility mode);

    // ---- identity: the core is the single source of truth (PROTOCOL.md §2) ----

    /// <summary><c>Q7KM-3PXA-…</c> DeviceID = BLAKE3(pubkey)[0..16]. Null before Start.</summary>
    string? DeviceId { get; }

    /// <summary>This device's six-word verification phrase. Null before Start.</summary>
    string? FingerprintPhrase { get; }

    /// <summary>Bound "ip:port" of the QUIC listener, for the mDNS TXT <c>p</c> field.</summary>
    string? ListenAddr { get; }

    /// <summary>
    /// The core's own phrase derivation for any peer key. Shells never reimplement the
    /// wordlist: a divergence here is a verification step the user cannot actually perform.
    /// </summary>
    string? FingerprintPhraseFor(byte[] publicKey);

    /// <summary>The core's own DeviceID derivation. Equals the peer id in every event.</summary>
    string? DeviceIdFor(byte[] publicKey);

    // ---- trust (PROTOCOL.md §4.5) ----

    /// <summary>
    /// The core's pinned peer set, read straight from its trust store. Re-read at launch,
    /// after every successful pairing, and after a revoke. Never mirrored.
    /// </summary>
    Task<IReadOnlyList<TrustedPeerInfo>> TrustedPeersAsync();

    /// <summary>Drops the core's pin. False when the key was not pinned to begin with.</summary>
    Task<bool> RevokePeerAsync(byte[] publicKey);

    // ---- pairing (PROTOCOL.md §4) ----

    /// <summary>Returns the <c>wooosh-pair:1?…</c> payload to render as a QR code.</summary>
    string BeginPairingQr();

    /// <summary>
    /// Parses a scanned or pasted payload locally: no network, no blocking. Null when the
    /// text is not a Wooosh pairing code at all.
    /// </summary>
    PairingCodeInfo? ParsePairingCode(string payload);

    /// <summary>
    /// Sender-side QR path. Blocking, and slow: address hints are raced but a dead network
    /// still costs the connect deadline, and the reply timeout is 20 s. The outcome always
    /// arrives as <see cref="CoreEvent.PairingResult"/>, including on failure, so the
    /// pairing UI is driven by the event and not by this call returning.
    /// </summary>
    Task PairWithQrAsync(string payload);

    /// <summary>Camera-less path: start SAS numeric comparison with a connected peer.</summary>
    void RequestSasPairing(string peerId);

    /// <summary>
    /// Confirms or rejects the six-digit comparison. Must be driven by a deliberate press
    /// and never bound to a default or Enter action.
    /// </summary>
    void ConfirmSas(string peerId, bool accepted);

    // ---- connections and transfers ----

    /// <summary>
    /// Connects to an address the native mDNS browser resolved (DESIGN.md §4).
    ///
    /// <paramref name="expectedPublicKey"/> pins the TLS handshake to that exact key;
    /// always pass the pinned key when the shell holds one. Passing null does not opt out
    /// of pinning: the core re-applies its own pin whenever it can resolve the identity
    /// behind the address (PROTOCOL.md §4.5). It only leaves the very first reconnect to a
    /// brand-new address unpinned.
    ///
    /// Returns the core's peer id, which is the peer's DeviceID.
    /// </summary>
    Task<string> ConnectPeerAsync(string addr, byte[]? expectedPublicKey = null);

    /// <summary>
    /// Begins an outgoing transfer. The resolved manifest arrives via TransferStarted.
    ///
    /// <paramref name="filePaths"/> are absolute paths the core reads directly. DESIGN.md §4
    /// sketches a richer <c>StagedFile</c> here, but the core's exported <c>send</c> takes
    /// plain paths and derives name and MIME itself, so the shell passes paths and never
    /// renames anything on the way (original filenames are preserved).
    /// </summary>
    Task<TransferId> SendAsync(string peerId, IReadOnlyList<string> filePaths);

    /// <summary>An empty <paramref name="acceptedFileIds"/> declines the whole offer.</summary>
    void RespondToOffer(TransferId transferId, IReadOnlyList<FileId> acceptedFileIds);

    /// <summary>Cancels a whole transfer, or one file when <paramref name="fileId"/> is given.</summary>
    void Cancel(TransferId transferId, FileId? fileId = null);
}

/// <summary>Thrown by core calls with a message the UI may show to the user as-is.</summary>
public sealed class CoreException : Exception
{
    public CoreException(string message, Exception? inner = null) : base(message, inner)
    {
    }
}

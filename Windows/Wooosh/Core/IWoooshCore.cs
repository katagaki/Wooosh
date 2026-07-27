namespace Wooosh.Core;

/// <summary>Shell-side seam over the wooosh-core FFI surface (DESIGN.md §4). Threading is
/// normative: every exported core call is synchronous and blocking and none may run on the
/// UI thread, so the blocking ones are Task-returning here.</summary>
public interface IWoooshCore : IDisposable
{
    /// <summary>Raised on the core's event thread; handlers must marshal to the UI thread.</summary>
    event Action<CoreEvent>? EventReceived;

    /// <summary>Blocking on the platform key store, possibly behind a Windows Hello prompt.</summary>
    Task StartAsync(CoreConfig config, CancellationToken cancellationToken = default);

    /// <summary>Blocking (~2 s of runtime shutdown plus joining the event thread).</summary>
    Task StopAsync();

    void SetVisibility(CoreVisibility mode);

    /// <summary>BLAKE3(pubkey)[0..16] (PROTOCOL.md §2). Null before Start.</summary>
    string? DeviceId { get; }

    string? FingerprintPhrase { get; }

    /// <summary>Bound "ip:port" of the QUIC listener, for the mDNS TXT <c>p</c> field.</summary>
    string? ListenAddr { get; }

    /// <summary>Shells never reimplement the wordlist: a divergence makes the verification
    /// step unperformable.</summary>
    string? FingerprintPhraseFor(byte[] publicKey);

    string? DeviceIdFor(byte[] publicKey);

    /// <summary>Read straight from the core's trust store (PROTOCOL.md §4.5), never mirrored.</summary>
    Task<IReadOnlyList<TrustedPeerInfo>> TrustedPeersAsync();

    /// <summary>False when the key was not pinned to begin with.</summary>
    Task<bool> RevokePeerAsync(byte[] publicKey);

    /// <summary>Returns the <c>wooosh-pair:1?…</c> payload (PROTOCOL.md §4).</summary>
    string BeginPairingQr();

    /// <summary>Local parse only, no network. Null when the text is not a Wooosh pairing code.</summary>
    PairingCodeInfo? ParsePairingCode(string payload);

    /// <summary>Blocking, 20 s reply timeout. Every outcome arrives as
    /// <see cref="CoreEvent.PairingResult"/>, so the UI is driven by the event.</summary>
    Task PairWithQrAsync(string payload);

    void RequestSasPairing(string peerId);

    /// <summary>Must be driven by a deliberate press, never bound to a default or Enter action.</summary>
    void ConfirmSas(string peerId, bool accepted);

    /// <summary><paramref name="expectedPublicKey"/> pins the TLS handshake to that key; null
    /// does not opt out, since the core re-applies its own pin whenever it can resolve the
    /// identity behind the address (PROTOCOL.md §4.5).</summary>
    Task<string> ConnectPeerAsync(string addr, byte[]? expectedPublicKey = null);

    /// <summary>Absolute paths the core reads directly, deriving name and MIME itself, so
    /// nothing is renamed on the way. The manifest arrives via TransferStarted.</summary>
    Task<TransferId> SendAsync(string peerId, IReadOnlyList<string> filePaths);

    /// <summary>An empty <paramref name="acceptedFileIds"/> declines the whole offer.</summary>
    void RespondToOffer(TransferId transferId, IReadOnlyList<FileId> acceptedFileIds);

    // The internet path (PROTOCOL.md §9). A ticket authorises exactly one transfer and dies
    // with it: nothing is paired and no fingerprint is shown, because there is no prior
    // relationship to check one against (DESIGN.md §9).

    /// <summary>Blocking: contacts the relay.</summary>
    Task<string> BeginInternetTicketAsync();

    /// <summary>Synchronous, and safe to call when there is no ticket.</summary>
    void EndInternetTicket();

    /// <summary>Blocking: hole punching takes longer than a LAN connection.</summary>
    Task<string> RedeemTicketAsync(string ticket);

    void Cancel(TransferId transferId, FileId? fileId = null);
}

/// <summary>Message is user-presentable as-is.</summary>
public sealed class CoreException : Exception
{
    public CoreException(string message, Exception? inner = null) : base(message, inner)
    {
    }
}

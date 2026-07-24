namespace Wooosh.Core;

/// <summary>
/// Event stream (core to shell), the single source of UI truth (DESIGN.md §4).
///
/// Delivered on the core's own event thread, never on the caller's, so every handler must
/// marshal to the UI thread before touching anything bound to XAML.
/// </summary>
public abstract record CoreEvent
{
    public sealed record PeerConnected(PeerRef Peer) : CoreEvent;

    public sealed record PeerDisconnected(string PeerId) : CoreEvent;

    /// <summary>Six-digit code for numeric comparison (PROTOCOL.md §4.3). Both sides show it.</summary>
    public sealed record PairingSas(PeerRef Peer, string SixDigits) : CoreEvent;

    /// <summary>
    /// Pairing concluded: QR or SAS, success or failure or timeout. Dismisses the pairing
    /// UI and refreshes the trust list. <paramref name="Peer"/> carries the key that was,
    /// or would have been, pinned.
    /// </summary>
    public sealed record PairingResult(
        string PeerId,
        PeerRef? Peer,
        bool Success,
        string? Message) : CoreEvent;

    public sealed record IncomingOffer(
        TransferId TransferId,
        PeerRef From,
        IReadOnlyList<FileMeta> Manifest) : CoreEvent;

    public sealed record TransferStarted(
        TransferId TransferId,
        PeerRef Peer,
        TransferDirection Direction,
        IReadOnlyList<FileMeta> Manifest) : CoreEvent;

    /// <param name="Rate">Bytes per second for this attempt.</param>
    /// <param name="EtaSeconds">Seconds remaining; -1 when the core cannot estimate one.</param>
    public sealed record Progress(
        TransferId TransferId,
        FileId FileId,
        long Bytes,
        long TotalBytes,
        long Rate,
        long EtaSeconds) : CoreEvent;

    /// <summary>
    /// The storage-routing hook (DESIGN.md §4 and §6): the core has verified the file in
    /// its private staging directory and the shell now moves it to Downloads.
    /// </summary>
    public sealed record FileReady(
        TransferId TransferId,
        FileId FileId,
        string StagedPath,
        FileKind Kind) : CoreEvent;

    public sealed record TransferDone(
        TransferId TransferId,
        int OkFiles,
        int FailedFiles,
        long BytesTransferred,
        long DurationMs) : CoreEvent;

    public sealed record TransferError(
        TransferId TransferId,
        string Message,
        bool Resumable) : CoreEvent;

    /// <summary>
    /// A pinned peer presented a different key (PROTOCOL.md §4.5). This is the MITM or
    /// reinstall signal and is a hard failure: it is surfaced prominently and never
    /// silently re-pinned. <paramref name="PresentedFingerprint"/> is null when the
    /// handshake failed before a key could be observed.
    /// </summary>
    public sealed record KeyChanged(
        PeerRef Peer,
        string? PresentedFingerprint) : CoreEvent;
}

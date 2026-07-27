namespace Wooosh.Core;

/// <summary>The single source of UI truth (DESIGN.md §4), delivered on the core's own event
/// thread: handlers must marshal to the UI thread before touching anything bound to XAML.</summary>
public abstract record CoreEvent
{
    public sealed record PeerConnected(PeerRef Peer) : CoreEvent;

    public sealed record PeerDisconnected(string PeerId) : CoreEvent;

    /// <summary>PROTOCOL.md §4.3. Both sides must show it, or the comparison is unperformable.</summary>
    public sealed record PairingSas(PeerRef Peer, string SixDigits) : CoreEvent;

    /// <summary>Raised for every outcome, QR or SAS, including failure and timeout.</summary>
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

    /// <param name="EtaSeconds">-1 when the core cannot estimate one.</param>
    public sealed record Progress(
        TransferId TransferId,
        FileId FileId,
        long Bytes,
        long TotalBytes,
        long Rate,
        long EtaSeconds) : CoreEvent;

    /// <summary>Storage-routing hook (DESIGN.md §4, §6): hash-verified in staging, and the
    /// shell now moves it to Downloads.</summary>
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

    /// <summary>A pinned peer presented a different key (PROTOCOL.md §4.5): a hard failure,
    /// never silently re-pinned. Null fingerprint when none was observed.</summary>
    public sealed record KeyChanged(
        PeerRef Peer,
        string? PresentedFingerprint) : CoreEvent;
}

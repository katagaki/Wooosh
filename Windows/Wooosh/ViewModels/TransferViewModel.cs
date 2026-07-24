using Wooosh.Core;
using Wooosh.Localization;

namespace Wooosh.ViewModels;

/// <summary>One transfer card: its header, its progress line and its per-file outcome.</summary>
public sealed partial class TransferViewModel : ObservableObject
{
    private readonly Dictionary<FileId, string> _names;
    private readonly long _totalBytes;

    private long _bytesTransferred;
    private long _rate;
    private long _etaSeconds = -1;
    private string _statusLine;
    private bool _isFinished;

    public TransferViewModel(
        TransferId id,
        PeerRef peer,
        TransferDirection direction,
        IReadOnlyList<FileMeta> manifest)
    {
        Id = id;
        Peer = peer;
        Direction = direction;
        FileCount = manifest.Count;
        _names = manifest.ToDictionary(file => file.Id, file => file.Name);
        _totalBytes = manifest.Sum(file => file.Size);
        _statusLine = Strings.Get("TransferPreparing");
    }

    public TransferId Id { get; }

    public PeerRef Peer { get; }

    public TransferDirection Direction { get; }

    public int FileCount { get; }

    /// <summary>"Sending to Kirsi" / "Receiving from Kirsi". One format string, no fragments.</summary>
    public string Header => Direction == TransferDirection.Send
        ? Strings.Format("TransferSendingTo", Peer.DisplayName)
        : Strings.Format("TransferReceivingFrom", Peer.DisplayName);

    public string StatusLine
    {
        get => _statusLine;
        private set => Set(ref _statusLine, value);
    }

    public bool IsFinished
    {
        get => _isFinished;
        private set => Set(ref _isFinished, value);
    }

    /// <summary>0 to 100 for the ProgressBar. Zero total means the bar stays indeterminate.</summary>
    public double PercentComplete =>
        _totalBytes <= 0 ? 0 : Math.Clamp(_bytesTransferred * 100.0 / _totalBytes, 0, 100);

    public string? NameOf(FileId fileId) => _names.GetValueOrDefault(fileId);

    public void ApplyProgress(CoreEvent.Progress progress)
    {
        _bytesTransferred = progress.Bytes;
        _rate = progress.Rate;
        _etaSeconds = progress.EtaSeconds;

        // One format string per line, filled positionally, so translators can reorder
        // freely (COPY_STYLE.md §5). Never concatenated sentence fragments.
        var line = Strings.Format(
            "TransferProgressBytes",
            Formatters.ByteSize(_bytesTransferred),
            Formatters.ByteSize(_totalBytes));

        if (_rate > 0)
        {
            line += "  ·  " + Strings.Format("TransferProgressRate", Formatters.ByteSize(_rate));
        }

        if (_etaSeconds >= 0)
        {
            line += "  ·  " + Strings.Format("TransferProgressEta", Formatters.Duration(_etaSeconds));
        }

        StatusLine = line;
        Raise(nameof(PercentComplete));
    }

    public void ApplyDone(CoreEvent.TransferDone done)
    {
        IsFinished = true;
        _bytesTransferred = done.BytesTransferred;

        StatusLine = done.FailedFiles switch
        {
            0 when Direction == TransferDirection.Send =>
                Strings.Plural("TransferDoneSent", done.OkFiles, done.OkFiles),
            0 => Strings.Plural("TransferDoneReceived", done.OkFiles, done.OkFiles),
            _ when done.OkFiles == 0 =>
                Strings.Plural("TransferDoneFailed", done.FailedFiles, done.FailedFiles),
            _ => Strings.Plural("TransferDoneReceived", done.OkFiles, done.OkFiles),
        };

        Raise(nameof(PercentComplete));
    }

    public void ApplyError(CoreEvent.TransferError error)
    {
        IsFinished = true;

        // The core's message is not shown verbatim: it is engine text, not user copy.
        StatusLine = error.Resumable
            ? Strings.Format("TransferResumable", Strings.Get("ErrorTransferFailed"))
            : Strings.Get("ErrorTransferFailed");
    }

    public void ApplyStored(FileId fileId, string finalPath)
    {
        _ = finalPath;
        _ = fileId;

        // Received files always land in Downloads on Windows (DESIGN.md §6), so the label
        // is the folder name and never a path: paths are not user-facing copy.
        StatusLine = Strings.Format(
            "TransferSavedTo",
            Formatters.ByteSize(_bytesTransferred),
            Strings.Get("StorageLocationDownloads"));
    }

    public void ApplyStoreFailure(FileId fileId)
    {
        _ = fileId;
        StatusLine = Strings.Get("ErrorSaveFailed");
    }

}

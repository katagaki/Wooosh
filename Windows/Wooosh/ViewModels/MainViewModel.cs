using System.Collections.ObjectModel;
using Microsoft.UI.Dispatching;
using Wooosh.Core;
using Wooosh.Discovery;
using Wooosh.Localization;
using Wooosh.Peers;
using Wooosh.Platform;
using Wooosh.Settings;

namespace Wooosh.ViewModels;

/// <summary>
/// Owns the app's long-lived pieces and turns the core event stream into UI state.
///
/// Everything the core raises arrives on the core's own thread, so every handler here
/// marshals to the UI thread before touching a bound collection (DESIGN.md §4).
/// </summary>
public sealed partial class MainViewModel : ObservableObject, IAsyncDisposable
{
    private readonly DispatcherQueue _dispatcher;
    private readonly IWoooshCore _core;
    private readonly DiscoveryController _discovery;
    private readonly Dictionary<TransferId, IDisposable> _wakeHolds = [];

    private string? _startupError;
    private string? _deviceId;
    private string? _fingerprintPhrase;

    public MainViewModel(DispatcherQueue dispatcher, IWoooshCore core, SettingsRepository settings)
    {
        _dispatcher = dispatcher;
        _core = core;
        Settings = settings;

        _discovery = new DiscoveryController(
            dispatcher,
            settings,
            // The core owns the QUIC socket; the shell only republishes the port it reports.
            listenPort: () => ParsePort(_core.ListenAddr));

        _core.EventReceived += OnCoreEvent;
    }

    public SettingsRepository Settings { get; }

    /// <summary>
    /// The core. Exposed so pages can make the cheap, non-blocking calls (BeginPairingQr,
    /// ConfirmSas, RespondToOffer) directly; anything blocking goes through a method on this
    /// view model so the off-UI-thread rule stays in one place.
    /// </summary>
    public IWoooshCore Core => _core;

    /// <summary>The device list. Append-only and never re-sorted; see <see cref="PeerRegistry"/>.</summary>
    public ObservableCollection<Peer> Peers => _discovery.Registry.Peers;

    /// <summary>Finished transfers and the ones in flight, newest first.</summary>
    public ObservableCollection<TransferViewModel> Transfers { get; } = [];

    /// <summary>
    /// Set when the core could not start. The message is already localized and safe to
    /// show: the technical reason goes to the debugger, never to the user.
    /// </summary>
    public string? StartupError
    {
        get => _startupError;
        private set => Set(ref _startupError, value);
    }

    public string? DeviceId
    {
        get => _deviceId;
        private set => Set(ref _deviceId, value);
    }

    public string? FingerprintPhrase
    {
        get => _fingerprintPhrase;
        private set => Set(ref _fingerprintPhrase, value);
    }

    /// <summary>Which line the empty state shows about who can see this device.</summary>
    public string VisibilityExplanation => Settings.Current.Visibility switch
    {
        CoreVisibility.Everyone => Strings.Get("EmptyVisibilityEveryone"),
        CoreVisibility.PairedOnly => Strings.Get("EmptyVisibilityPaired"),
        _ => Strings.Get("EmptyVisibilityOff"),
    };

    public async Task StartAsync()
    {
        var settings = Settings.Current;
        var localFolder = Windows.Storage.ApplicationData.Current.LocalFolder.Path;

        var config = new CoreConfig
        {
            DisplayName = settings.DisplayName,
            DeviceType = DeviceType.Windows,
            Visibility = settings.Visibility,
            StagingDir = Path.Combine(localFolder, "staging"),
            TrustStorePath = Path.Combine(localFolder, "trust.json"),
        };

        Directory.CreateDirectory(config.StagingDir);

        try
        {
            await _core.StartAsync(config);
            DeviceId = _core.DeviceId;
            FingerprintPhrase = _core.FingerprintPhrase;
            StartupError = null;
        }
        catch (CoreException e)
        {
            System.Diagnostics.Debug.WriteLine($"[Wooosh] core start failed: {e.InnerException?.Message ?? e.Message}");
            StartupError = e.Message;
        }

        // Discovery is native and independent of the core (DESIGN.md §2), so it runs even
        // when the core did not come up: the device list is real either way, and Wooosh
        // says plainly that it cannot transfer rather than showing an empty screen.
        _discovery.Start();
    }

    /// <summary>Pull to refresh. The only thing that clears the device list.</summary>
    public void Refresh() => _discovery.Refresh();

    public void SetVisibility(CoreVisibility visibility)
    {
        Settings.SetVisibility(visibility);
        Raise(nameof(VisibilityExplanation));
        try
        {
            _core.SetVisibility(visibility);
        }
        catch (CoreException)
        {
            // The advertiser still honours it, so Wooosh at least stops announcing.
        }
    }

    private void OnCoreEvent(CoreEvent coreEvent) => _dispatcher.TryEnqueue(() =>
    {
        switch (coreEvent)
        {
            case CoreEvent.PeerConnected e:
                _discovery.Registry.NoteConnected(e.Peer.Id, e.Peer.DisplayName, e.Peer.DeviceType, e.Peer.Paired);
                break;

            case CoreEvent.TransferStarted e:
                // Keep the machine awake for as long as bytes are moving (DESIGN.md §7).
                _wakeHolds[e.TransferId] = PowerManagement.Acquire();
                Transfers.Insert(0, new TransferViewModel(e.TransferId, e.Peer, e.Direction, e.Manifest));
                break;

            case CoreEvent.Progress e:
                Find(e.TransferId)?.ApplyProgress(e);
                break;

            case CoreEvent.FileReady e:
                _ = RouteAsync(e);
                break;

            case CoreEvent.TransferDone e:
                ReleaseWakeHold(e.TransferId);
                Find(e.TransferId)?.ApplyDone(e);
                break;

            case CoreEvent.TransferError e:
                ReleaseWakeHold(e.TransferId);
                Find(e.TransferId)?.ApplyError(e);
                break;

            // TODO(views): IncomingOffer, PairingSas, PairingResult and KeyChanged each own
            // a dialog. The dialogs exist under Views/; wiring them needs the core actually
            // delivering these events, which is blocked on the bindings.
            default:
                break;
        }
    });

    /// <summary>
    /// Storage routing (DESIGN.md §6). The file is only reported as received once it is in
    /// Downloads: a transfer is never called complete for a file the user cannot find.
    /// </summary>
    private async Task RouteAsync(CoreEvent.FileReady ready)
    {
        var transfer = Find(ready.TransferId);
        var name = transfer?.NameOf(ready.FileId) ?? Path.GetFileName(ready.StagedPath);

        try
        {
            var finalPath = await Task.Run(
                () => StorageRouter.RouteToDownloads(ready.StagedPath, name));
            _dispatcher.TryEnqueue(() => transfer?.ApplyStored(ready.FileId, finalPath));
        }
        catch (Exception e)
        {
            System.Diagnostics.Debug.WriteLine($"[Wooosh] routing {name} failed: {e.Message}");
            _dispatcher.TryEnqueue(() => transfer?.ApplyStoreFailure(ready.FileId));
        }
    }

    private TransferViewModel? Find(TransferId id) =>
        Transfers.FirstOrDefault(transfer => transfer.Id.Equals(id));

    private void ReleaseWakeHold(TransferId id)
    {
        if (_wakeHolds.Remove(id, out var hold))
        {
            hold.Dispose();
        }
    }

    /// <summary>Extracts the port from the core's "ip:port", including the IPv6 form.</summary>
    private static int ParsePort(string? listenAddr)
    {
        if (string.IsNullOrEmpty(listenAddr))
        {
            return 0;
        }

        var separator = listenAddr.LastIndexOf(':');
        return separator >= 0 && int.TryParse(listenAddr[(separator + 1)..], out var port) ? port : 0;
    }

    public async ValueTask DisposeAsync()
    {
        _core.EventReceived -= OnCoreEvent;
        foreach (var hold in _wakeHolds.Values)
        {
            hold.Dispose();
        }

        _wakeHolds.Clear();
        await _discovery.DisposeAsync();
        await _core.StopAsync();
        _core.Dispose();
    }
}

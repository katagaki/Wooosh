using System.Security.Cryptography;
using Microsoft.UI.Dispatching;
using Wooosh.Core;
using Wooosh.Peers;
using Wooosh.Settings;

namespace Wooosh.Discovery;

/// <summary><c>listenPort</c> is the core's bound QUIC port, published as the TXT <c>p</c>
/// field (DESIGN.md §4). Nothing is advertised until it is non-zero: announcing a port the
/// core is not listening on produces a device that answers nothing.</summary>
public sealed class DiscoveryController : IAsyncDisposable
{
    /// <summary>Display-name edits arrive per keystroke; re-registering on each one makes the
    /// device flicker in and out of every other device's list.</summary>
    private const int ReRegisterDebounceMs = 500;

    private readonly SettingsRepository _settings;
    private readonly Func<int> _listenPort;
    private readonly DnssdAdvertiser _advertiser = new();
    private readonly DnssdBrowser _browser;
    private readonly DispatcherQueueTimer _debounce;

    private bool _started;

    /// <summary>Rotating discovery ID (PROTOCOL.md §3.1), regenerated per process and
    /// deliberately not derived from the identity key: that is what stops a passive listener
    /// tracking this machine across networks.</summary>
    public string Rid { get; } = Convert.ToHexString(RandomNumberGenerator.GetBytes(8)).ToLowerInvariant();

    public PeerRegistry Registry { get; }

    public DiscoveryController(
        DispatcherQueue dispatcher,
        SettingsRepository settings,
        Func<int> listenPort)
    {
        _settings = settings;
        _listenPort = listenPort;
        Registry = new PeerRegistry(dispatcher);
        _browser = new DnssdBrowser(Registry, () => Rid, () => _advertiser.RegisteredInstanceName);

        _debounce = dispatcher.CreateTimer();
        _debounce.Interval = TimeSpan.FromMilliseconds(ReRegisterDebounceMs);
        _debounce.IsRepeating = false;
        _debounce.Tick += (_, _) => _ = ReadvertiseAsync();
    }

    public void Start()
    {
        if (_started)
        {
            return;
        }

        _started = true;
        _browser.Start();
        _settings.Changed += OnSettingsChanged;
        Readvertise();
    }

    public void Readvertise()
    {
        _debounce.Stop();
        _debounce.Start();
    }

    /// <summary>The only in-process way the append-only device list is cleared (DESIGN.md §5).</summary>
    public void Refresh()
    {
        Registry.Clear();
        _browser.Restart();
    }

    private void OnSettingsChanged() => Readvertise();

    private Task ReadvertiseAsync()
    {
        var settings = _settings.Current;
        var record = new TxtRecord
        {
            Rid = Rid,
            DisplayName = settings.DisplayName,
            DeviceType = DeviceType.Windows,
            Port = _listenPort(),
            Visibility = settings.Visibility,
        };

        return _advertiser.ApplyAsync(record);
    }

    public async ValueTask DisposeAsync()
    {
        _settings.Changed -= OnSettingsChanged;
        _debounce.Stop();
        _browser.Dispose();
        Registry.Dispose();
        await _advertiser.DisposeAsync();
    }
}

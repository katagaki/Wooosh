using System.Diagnostics;
using System.Net;
using Windows.Devices.Enumeration;
using Wooosh.Peers;

namespace Wooosh.Discovery;

/// <summary>Two deliberate mechanisms: a long-lived <see cref="DeviceWatcher"/> so a new
/// device appears immediately, and a 2 s poll (PROTOCOL.md §3.3) that keeps producing
/// sightings for a peer just sitting there, which Updated events do not. Nothing is ever
/// removed and Removed events are ignored: the registry's 10 s silence timeout is the single
/// stale rule, and a second faster path would defeat it.</summary>
public sealed class DnssdBrowser : IDisposable
{
    private const string DnssdProtocolId = "{4526e8c1-8aac-4153-9b16-55e86ada0e54}";

    private const int PollIntervalMs = 2_000;

    private static readonly string[] RequestedProperties =
    [
        "System.Devices.Dnssd.HostName",
        "System.Devices.Dnssd.InstanceName",
        "System.Devices.Dnssd.PortNumber",
        "System.Devices.Dnssd.ServiceName",
        "System.Devices.Dnssd.TextAttributes",
        "System.Devices.IpAddress",
    ];

    private static readonly string Query =
        $"System.Devices.AepService.ProtocolId:=\"{DnssdProtocolId}\" AND " +
        $"System.Devices.Dnssd.ServiceName:=\"{DnssdAdvertiser.ServiceType}\" AND " +
        "System.Devices.Dnssd.Domain:=\"local\"";

    private readonly PeerRegistry _registry;
    private readonly Func<string?> _ownRid;
    private readonly Func<string?> _ownInstanceName;

    private DeviceWatcher? _watcher;
    private CancellationTokenSource? _polling;

    public DnssdBrowser(PeerRegistry registry, Func<string?> ownRid, Func<string?> ownInstanceName)
    {
        _registry = registry;
        _ownRid = ownRid;
        _ownInstanceName = ownInstanceName;
    }

    public void Start()
    {
        if (_watcher is not null)
        {
            return;
        }

        _watcher = DeviceInformation.CreateWatcher(
            Query,
            RequestedProperties,
            DeviceInformationKind.AssociationEndpointService);

        _watcher.Added += (_, info) => Ingest(info.Name, info.Properties);
        _watcher.Updated += (_, update) => Ingest(null, update.Properties);
        _watcher.Start();

        _polling = new CancellationTokenSource();
        _ = PollAsync(_polling.Token);
    }

    public void Restart()
    {
        Stop();
        Start();
    }

    public void Stop()
    {
        _polling?.Cancel();
        _polling?.Dispose();
        _polling = null;

        if (_watcher is not null)
        {
            if (_watcher.Status is DeviceWatcherStatus.Started or DeviceWatcherStatus.EnumerationCompleted)
            {
                _watcher.Stop();
            }

            _watcher = null;
        }
    }

    private async Task PollAsync(CancellationToken cancellationToken)
    {
        using var timer = new PeriodicTimer(TimeSpan.FromMilliseconds(PollIntervalMs));
        try
        {
            while (await timer.WaitForNextTickAsync(cancellationToken))
            {
                var found = await DeviceInformation.FindAllAsync(
                    Query,
                    RequestedProperties,
                    DeviceInformationKind.AssociationEndpointService);

                foreach (var info in found)
                {
                    Ingest(info.Name, info.Properties);
                }
            }
        }
        catch (OperationCanceledException)
        {
            // Stop() was called.
        }
        catch (Exception e)
        {
            // A failed scan must not kill the loop; the next tick tries again.
            Debug.WriteLine($"[Wooosh] mDNS scan failed: {e.Message}");
        }
    }

    private void Ingest(string? name, IReadOnlyDictionary<string, object> properties)
    {
        var attributes = properties.GetValueOrDefault("System.Devices.Dnssd.TextAttributes") as string[];
        if (attributes is null || attributes.Length == 0)
        {
            return;
        }

        var instanceName =
            properties.GetValueOrDefault("System.Devices.Dnssd.InstanceName") as string
            ?? name
            ?? string.Empty;

        var record = TxtRecord.Parse(attributes, instanceName);
        if (record is null)
        {
            return;
        }

        // Do not list ourselves. The instance-name guard uses StartsWith because the browser
        // reports a bare label while the registration reports the fully qualified name.
        var ownInstanceName = _ownInstanceName();
        if (record.Rid == _ownRid() ||
            (instanceName.Length > 0 &&
             ownInstanceName is not null &&
             ownInstanceName.StartsWith(instanceName, StringComparison.Ordinal)))
        {
            return;
        }

        var addresses = ParseAddresses(properties.GetValueOrDefault("System.Devices.IpAddress"));

        // The TXT `p` is normative where it disagrees with the SRV port (PROTOCOL.md §3.1).
        _registry.NoteSighting(record.Rid, record.DisplayName, record.DeviceType, record.Port, addresses);
    }

    /// <summary>IPv4 first: it is what routes on most home LANs.</summary>
    private static IReadOnlyList<IPAddress> ParseAddresses(object? value)
    {
        if (value is not string[] raw)
        {
            return [];
        }

        return [.. raw
            .Select(text => IPAddress.TryParse(text, out var address) ? address : null)
            .Where(address => address is not null)
            .Select(address => address!)
            .OrderBy(address => address.AddressFamily == System.Net.Sockets.AddressFamily.InterNetwork ? 0 : 1)];
    }

    public void Dispose() => Stop();
}

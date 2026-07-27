using System.Diagnostics;
using Windows.Networking.ServiceDiscovery.Dnssd;
using Windows.Networking.Sockets;
using Wooosh.Core;
using Wooosh.Peers;

namespace Wooosh.Discovery;

/// <summary>Advertises the TXT layout of PROTOCOL.md §3.1 via the WinRT DNS-SD API, so no
/// second mDNS responder fights the system one (DESIGN.md §2). <c>DnssdServiceInstance</c>
/// can only register against a socket listener and takes the SRV port from it, but Wooosh's
/// QUIC/UDP listener is owned by the core; TCP and UDP port numbers are separate namespaces,
/// so the <c>StreamSocketListener</c> here does not collide and accepts nothing.</summary>
public sealed class DnssdAdvertiser : IAsyncDisposable
{
    /// <summary>PROTOCOL.md §1. The <c>_tcp</c> is DNS-SD convention, not the transport.</summary>
    public const string ServiceType = "_wooosh._tcp";

    private const string Domain = "local";

    private StreamSocketListener? _listener;
    private DnssdServiceInstance? _instance;

    /// <summary>What the OS registered, which differs from the request when mDNS resolves a
    /// name conflict. The browser needs the real one to filter out this device.</summary>
    public string? RegisteredInstanceName { get; private set; }

    /// <summary>Tears the old registration down first: this API cannot update TXT in place.</summary>
    public async Task ApplyAsync(TxtRecord record)
    {
        await StopAsync();

        // Visibility Off means no announcements and no listener at all (DESIGN.md §10).
        if (record.Visibility == CoreVisibility.Off || record.Port <= 0)
        {
            return;
        }

        // "<DisplayName> (<4-char suffix of rid>)._wooosh._tcp.local" (PROTOCOL.md §3.1).
        var suffix = record.Rid.Length >= 4 ? record.Rid[^4..] : record.Rid;
        var instanceName = $"{record.DisplayName} ({suffix}).{ServiceType}.{Domain}";

        var instance = new DnssdServiceInstance(instanceName, hostName: null, port: (ushort)record.Port);
        foreach (var (key, value) in record.ToAttributes())
        {
            instance.TextAttributes[key] = value;
        }

        var listener = new StreamSocketListener();
        try
        {
            var result = await instance.RegisterStreamSocketListenerAsync(listener);
            if (result.Status != DnssdRegistrationStatus.Success)
            {
                // Not fatal: Wooosh can still see other devices and send to them while invisible.
                Debug.WriteLine($"[Wooosh] mDNS registration failed: {result.Status}");
                listener.Dispose();
                return;
            }

            _listener = listener;
            _instance = instance;
            RegisteredInstanceName = instance.DnssdServiceInstanceName;
        }
        catch (Exception e)
        {
            Debug.WriteLine($"[Wooosh] mDNS registration threw: {e.Message}");
            listener.Dispose();
        }
    }

    public Task StopAsync()
    {
        _listener?.Dispose();
        _listener = null;
        _instance = null;
        RegisteredInstanceName = null;
        return Task.CompletedTask;
    }

    public async ValueTask DisposeAsync() => await StopAsync();
}

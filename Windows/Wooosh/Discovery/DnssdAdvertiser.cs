using System.Diagnostics;
using Windows.Networking.ServiceDiscovery.Dnssd;
using Windows.Networking.Sockets;
using Wooosh.Core;
using Wooosh.Peers;

namespace Wooosh.Discovery;

/// <summary>
/// Advertises this device over mDNS/DNS-SD with the TXT layout of PROTOCOL.md §3.1.
///
/// <para><b>Why the WinRT DNS-SD API.</b> Discovery is native per platform (DESIGN.md §2)
/// precisely so that no shell ships a second mDNS responder to fight the system one.
/// Windows has had an mDNS responder in <c>dnsapi.dll</c> since Windows 10 1703, and
/// <c>Windows.Networking.ServiceDiscovery.Dnssd</c> is its public surface, so it is the
/// direct counterpart of <c>NsdManager</c> on Android and <c>NWListener</c> on Apple.
/// A hand-rolled responder on <c>System.Net.Sockets</c> would mean binding UDP 5353
/// alongside the OS responder and re-implementing DNS record encoding, which is a lot of
/// unverifiable code for a service the OS already publishes correctly. A third-party mDNS
/// NuGet package has the same problem plus a dependency.</para>
///
/// <para><b>The one wart.</b> <c>DnssdServiceInstance</c> can only be registered against a
/// <c>StreamSocketListener</c> or a <c>DatagramSocket</c>, and the SRV port comes from the
/// socket. Wooosh's listener is QUIC over UDP, owned by the core, so the port the core
/// reports has to be published without this shell owning a socket on it. TCP and UDP port
/// numbers are separate namespaces, so a <c>StreamSocketListener</c> bound to the same
/// number does not collide with the core's UDP socket, and DNS-SD convention already puts
/// the QUIC UDP port in an <c>_tcp</c> SRV record (PROTOCOL.md §1). The listener accepts
/// nothing: it exists so the OS has something to attach the registration to.</para>
/// </summary>
public sealed class DnssdAdvertiser : IAsyncDisposable
{
    /// <summary>PROTOCOL.md §1. The <c>_tcp</c> is DNS-SD convention, not the transport.</summary>
    public const string ServiceType = "_wooosh._tcp";

    private const string Domain = "local";

    private StreamSocketListener? _listener;
    private DnssdServiceInstance? _instance;

    /// <summary>
    /// The instance name the OS actually registered. It differs from what was requested
    /// when mDNS resolves a name conflict, and the browser needs the real one to filter
    /// this device out of its own results.
    /// </summary>
    public string? RegisteredInstanceName { get; private set; }

    /// <summary>
    /// Republishes the record. Safe to call on every settings change: it tears the old
    /// registration down first, because a TXT update is not something DNS-SD lets you do
    /// in place through this API.
    /// </summary>
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
                // Losing discovery is bad. Taking the app down with it is worse: Wooosh can
                // still see other devices and send to them while invisible.
                Debug.WriteLine($"[Wooosh] mDNS registration failed: {result.Status}");
                listener.Dispose();
                return;
            }

            _listener = listener;
            _instance = instance;
            // mDNS may have resolved a name conflict by renaming us; take what it registered.
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

using System.Collections.Generic;
using System.ComponentModel;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Runtime.CompilerServices;
using Wooosh.Localization;

namespace Wooosh.Peers;

/// <summary>
/// One row of the device list, as discovered over mDNS (PROTOCOL.md §3.1).
///
/// Observable and mutated in place on purpose. <see cref="PeerRegistry"/> holds these in
/// an append-only <c>ObservableCollection</c>, so removing and re-inserting a row to
/// change its name or its stale flag would move it, and rows must never move
/// (DESIGN.md §5). Every field the UI shows therefore raises PropertyChanged instead.
/// </summary>
public sealed partial class Peer : INotifyPropertyChanged
{
    /// <summary>Rotating discovery ID (PROTOCOL.md §3.1) and the de-duplication key.</summary>
    public required string Rid { get; init; }

    /// <summary>
    /// <c>Environment.TickCount64</c> of the FIRST sighting in this process. The permanent
    /// ordering key: written once, never again, not even when the peer goes away and
    /// comes back. A monotonic clock, so it cannot jump when the system clock is set.
    /// </summary>
    public required long DiscoveredAtTicks { get; init; }

    private string _displayName = string.Empty;
    public string DisplayName
    {
        get => _displayName;
        set
        {
            if (Set(ref _displayName, value))
            {
                Raise(nameof(Label));
            }
        }
    }

    /// <summary>What the row shows. A device that advertises no name still needs a label.</summary>
    public string Label => string.IsNullOrWhiteSpace(_displayName)
        ? Strings.Get("PeerUnnamed")
        : _displayName;

    private DeviceType _deviceType = DeviceType.Unknown;
    public DeviceType DeviceType
    {
        get => _deviceType;
        set
        {
            if (Set(ref _deviceType, value))
            {
                Raise(nameof(Glyph));
                Raise(nameof(DeviceKindKey));
            }
        }
    }

    /// <summary>QUIC UDP port from the TXT <c>p</c> field.</summary>
    public int Port { get; set; }

    /// <summary>Addresses mDNS resolved for this instance, IPv4 first.</summary>
    public IReadOnlyList<IPAddress> Addresses { get; set; } = [];

    /// <summary><c>Environment.TickCount64</c> of the most recent announce.</summary>
    public long LastSeenTicks { get; set; }

    private bool _isStale;
    /// <summary>True once the peer has been silent past the 10 s threshold.</summary>
    public bool IsStale
    {
        get => _isStale;
        set
        {
            if (Set(ref _isStale, value))
            {
                Raise(nameof(IsAvailable));
                Raise(nameof(State));
            }
        }
    }

    private bool _isPaired;
    /// <summary>Mirrors the core's trust store; never a shell-side second copy of it.</summary>
    public bool IsPaired
    {
        get => _isPaired;
        set
        {
            if (Set(ref _isPaired, value))
            {
                Raise(nameof(State));
            }
        }
    }

    /// <summary>
    /// The row's state line: Ready, Ready · Paired, or Away. A paired checkmark rather than
    /// a separate section, because sections would move rows (DESIGN.md §5).
    /// </summary>
    public string State => IsStale
        ? Strings.Get("PeerStateAway")
        : IsPaired
            ? Strings.Get("PeerStateReadyPaired")
            : Strings.Get("PeerStateReady");

    /// <summary>Bound directly by the row: a stale row is disabled, not removed.</summary>
    public bool IsAvailable => !IsStale;

    public string Glyph => DeviceType.ToGlyph();

    public string DeviceKindKey => DeviceType.ToAccessibleNameKey();

    /// <summary>
    /// The core's peer id (the peer's DeviceID) once a connection to this row has been
    /// established in this session. Lets a repeat send pass the pinned key to connect_peer.
    /// </summary>
    public string? PeerId { get; set; }

    /// <summary>
    /// "host:port" for connect_peer, or null when nothing has resolved yet. IPv6 literals
    /// are bracketed, which the core's SocketAddr parser requires.
    /// </summary>
    public string? Address
    {
        get
        {
            var host = Addresses.FirstOrDefault();
            if (host is null || Port <= 0)
            {
                return null;
            }

            return host.AddressFamily == AddressFamily.InterNetworkV6
                ? $"[{host}]:{Port}"
                : $"{host}:{Port}";
        }
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    private bool Set<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
        {
            return false;
        }

        field = value;
        Raise(name);
        return true;
    }

    private void Raise(string? name) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}

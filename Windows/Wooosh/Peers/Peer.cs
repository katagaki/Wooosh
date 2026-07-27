using System.Collections.Generic;
using System.ComponentModel;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Runtime.CompilerServices;
using Wooosh.Localization;

namespace Wooosh.Peers;

/// <summary>Mutated in place, never re-inserted: rows must never move (DESIGN.md §5).</summary>
public sealed partial class Peer : INotifyPropertyChanged
{
    /// <summary>Rotating discovery ID (PROTOCOL.md §3.1) and the de-duplication key.</summary>
    public required string Rid { get; init; }

    /// <summary>The permanent ordering key: written once, on a monotonic clock that cannot jump.</summary>
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

    public int Port { get; set; }

    public IReadOnlyList<IPAddress> Addresses { get; set; } = [];

    public long LastSeenTicks { get; set; }

    private bool _isStale;
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

    /// <summary>Paired is shown inline, not as a section: sections would move rows (DESIGN.md §5).</summary>
    public string State => IsStale
        ? Strings.Get("PeerStateAway")
        : IsPaired
            ? Strings.Get("PeerStateReadyPaired")
            : Strings.Get("PeerStateReady");

    /// <summary>A stale row is disabled, not removed.</summary>
    public bool IsAvailable => !IsStale;

    public string Glyph => DeviceType.ToGlyph();

    public string DeviceKindKey => DeviceType.ToAccessibleNameKey();

    public string? PeerId { get; set; }

    /// <summary>IPv6 literals are bracketed, which the core's SocketAddr parser requires.</summary>
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

using System.Collections.ObjectModel;
using System.Net;
using Microsoft.UI.Dispatching;

namespace Wooosh.Peers;

/// <summary>
/// The canonical device list (DESIGN.md §5, PROTOCOL.md §3.3). Ordered by first sighting,
/// append-only, never re-sorted; a peer that stops advertising greys out in place and is
/// never removed, because rows shifting under the cursor is how a file reaches the wrong
/// device. Only an explicit user refresh clears the list.
/// </summary>
public sealed class PeerRegistry : IDisposable
{
    /// <summary>10 s, PROTOCOL.md §3.3, and it stays 10 s. Do not tune.</summary>
    public const int StaleThresholdMs = 10_000;

    private const int SweepIntervalMs = 1_000;

    private readonly DispatcherQueue _dispatcher;
    private readonly DispatcherQueueTimer _sweepTimer;
    private readonly Dictionary<string, Peer> _byRid = [];

    public ObservableCollection<Peer> Peers { get; } = [];

    public PeerRegistry(DispatcherQueue dispatcher)
    {
        _dispatcher = dispatcher;
        _sweepTimer = dispatcher.CreateTimer();
        _sweepTimer.Interval = TimeSpan.FromMilliseconds(SweepIntervalMs);
        _sweepTimer.IsRepeating = true;
        _sweepTimer.Tick += (_, _) => Sweep();
        _sweepTimer.Start();
    }

    /// <summary>Callable from any thread: ObservableCollection notifications must be raised on the UI one.</summary>
    public void NoteSighting(
        string rid,
        string displayName,
        DeviceType deviceType,
        int port,
        IReadOnlyList<IPAddress> addresses)
    {
        _dispatcher.TryEnqueue(() =>
        {
            var now = Environment.TickCount64;

            if (_byRid.TryGetValue(rid, out var existing))
            {
                existing.LastSeenTicks = now;
                existing.DisplayName = string.IsNullOrWhiteSpace(displayName)
                    ? existing.DisplayName
                    : displayName;
                existing.DeviceType = deviceType;
                if (port > 0)
                {
                    existing.Port = port;
                }

                // A TXT-only refresh can carry no addresses; keep the resolved ones.
                if (addresses.Count > 0)
                {
                    existing.Addresses = addresses;
                }

                existing.IsStale = false;
                return;
            }

            var peer = new Peer
            {
                Rid = rid,
                DiscoveredAtTicks = now,
                LastSeenTicks = now,
                DisplayName = displayName,
                DeviceType = deviceType,
                Port = port,
                Addresses = addresses,
                IsStale = false,
            };

            _byRid[rid] = peer;

            // Never Insert, never Sort: DiscoveredAtTicks is monotonic, so append order is discovery order.
            Peers.Add(peer);
        });
    }

    /// <summary>HELLO's authenticated name beats the TXT hint; Unknown never overwrites a known type.</summary>
    public void NoteConnected(string peerId, string displayName, DeviceType deviceType, bool trusted)
    {
        _dispatcher.TryEnqueue(() =>
        {
            foreach (var peer in Peers)
            {
                if (peer.PeerId != peerId)
                {
                    continue;
                }

                if (!string.IsNullOrWhiteSpace(displayName))
                {
                    peer.DisplayName = displayName;
                }

                if (deviceType != DeviceType.Unknown)
                {
                    peer.DeviceType = deviceType;
                }

                peer.IsPaired = trusted;
            }
        });
    }

    public void AttachPeerId(string rid, string peerId)
    {
        _dispatcher.TryEnqueue(() =>
        {
            if (_byRid.TryGetValue(rid, out var peer))
            {
                peer.PeerId = peerId;
            }
        });
    }

    public void ApplyTrust(IReadOnlyCollection<string> trustedPeerIds)
    {
        _dispatcher.TryEnqueue(() =>
        {
            foreach (var peer in Peers)
            {
                peer.IsPaired = peer.PeerId is not null && trustedPeerIds.Contains(peer.PeerId);
            }
        });
    }

    /// <summary>Only ever reached from an explicit user refresh (DESIGN.md §5).</summary>
    public void Clear()
    {
        _dispatcher.TryEnqueue(() =>
        {
            _byRid.Clear();
            Peers.Clear();
        });
    }

    /// <summary>Runs on the UI thread (DispatcherQueueTimer), so it can touch the items directly.</summary>
    private void Sweep()
    {
        var now = Environment.TickCount64;
        foreach (var peer in Peers)
        {
            var silent = now - peer.LastSeenTicks >= StaleThresholdMs;
            if (peer.IsStale != silent)
            {
                peer.IsStale = silent;
            }
        }
    }

    public void Dispose() => _sweepTimer.Stop();
}

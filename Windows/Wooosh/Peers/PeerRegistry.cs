using System.Collections.ObjectModel;
using System.Net;
using Microsoft.UI.Dispatching;

namespace Wooosh.Peers;

/// <summary>
/// The canonical device list (DESIGN.md §5, PROTOCOL.md §3.3). These rules are
/// non-negotiable and this class is the only place that enforces them:
///
/// <list type="bullet">
/// <item>Rows are ordered strictly by the tick count of their FIRST sighting in this
/// process. New devices append to the bottom.</item>
/// <item>The collection is append-only and is NEVER re-sorted. Not on rename, not on
/// re-announce, not on pairing, not on going stale.</item>
/// <item>A peer that stops advertising is greyed out in place, disabled, and keeps its
/// position and height. It is never removed. Rows shifting under a cursor is how a
/// user sends a file to the wrong device.</item>
/// <item>The stale threshold is 10 s and stays 10 s. Scanning happens every 2 s, so that
/// is roughly five missed announces rather than two. Faster scanning finds devices
/// sooner; it does not drop them sooner.</item>
/// </list>
///
/// The only thing that clears the list is an explicit user refresh (or relaunch).
/// </summary>
public sealed class PeerRegistry : IDisposable
{
    /// <summary>PROTOCOL.md §3.3. Do not "tune" this.</summary>
    public const int StaleThresholdMs = 10_000;

    private const int SweepIntervalMs = 1_000;

    private readonly DispatcherQueue _dispatcher;
    private readonly DispatcherQueueTimer _sweepTimer;
    private readonly Dictionary<string, Peer> _byRid = [];

    /// <summary>
    /// Bound by the device list with x:Bind. Append-only: <see cref="Peers"/> is only ever
    /// added to, or cleared wholesale by <see cref="Clear"/>.
    /// </summary>
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

    /// <summary>
    /// An announce for <paramref name="rid"/> arrived (mDNS browse result, PROTOCOL.md §3.1).
    /// Safe to call from any thread: it marshals to the UI thread, because
    /// ObservableCollection change notifications must be raised there.
    /// </summary>
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

                // Re-enabled in place. DiscoveredAtTicks, the ordering key, is untouched.
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

            // Append. Never Insert, never Sort: DiscoveredAtTicks is monotonic, so append
            // order IS discovery order, and the list stays correct without ever being
            // reordered.
            Peers.Add(peer);
        });
    }

    /// <summary>
    /// A control channel came up (HELLO, PROTOCOL.md §4.1). The name in HELLO is
    /// authenticated and therefore beats the mDNS TXT, which is untrusted hint material.
    ///
    /// A device type of <see cref="DeviceType.Unknown"/> never overwrites a known one:
    /// replacing a correct <c>android-phone</c> glyph from the TXT record with a neutral
    /// one because HELLO said nothing is a regression, not a correction.
    /// </summary>
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

    /// <summary>
    /// Records the core's peer id for a row after a successful connect, so a later send to
    /// the same row can pass the pinned key to connect_peer.
    /// </summary>
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

    /// <summary>Marks rows paired or unpaired from the core's trust store.</summary>
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

    /// <summary>
    /// The only in-process way the list is emptied, and it exists solely because
    /// DESIGN.md §5 allows an explicit user refresh to do it.
    /// </summary>
    public void Clear()
    {
        _dispatcher.TryEnqueue(() =>
        {
            _byRid.Clear();
            Peers.Clear();
        });
    }

    /// <summary>
    /// Greys out rows that have been silent past the threshold. Runs on the UI thread
    /// (DispatcherQueueTimer), so it can touch the collection's items directly.
    /// </summary>
    private void Sweep()
    {
        var now = Environment.TickCount64;
        foreach (var peer in Peers)
        {
            var silent = now - peer.LastSeenTicks >= StaleThresholdMs;
            if (peer.IsStale != silent)
            {
                // In place. The row keeps its index, and the ListView keeps its scroll.
                peer.IsStale = silent;
            }
        }
    }

    public void Dispose() => _sweepTimer.Stop();
}

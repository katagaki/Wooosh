using Wooosh.Peers;

namespace Wooosh.Core;

/// <summary>Transfer id as the core hands it out. Opaque to the shell.</summary>
public readonly record struct TransferId(string Value)
{
    public override string ToString() => Value;
}

/// <summary>File id inside a transfer: <c>fid</c> on the wire (PROTOCOL.md §5), u32 in the core.</summary>
public readonly record struct FileId(uint Value)
{
    public override string ToString() => Value.ToString();
}

public enum CoreVisibility
{
    Everyone,
    PairedOnly,
    Off,
}

public static class CoreVisibilityExtensions
{
    /// <summary>The <c>vis</c> TXT value of PROTOCOL.md §3.1. <c>Off</c> does not advertise at all.</summary>
    public static string? ToTxtValue(this CoreVisibility visibility) => visibility switch
    {
        CoreVisibility.Everyone => "e",
        CoreVisibility.PairedOnly => "p",
        _ => null,
    };
}

public enum FileKind
{
    Photo,
    Video,
    Document,
}

public enum TransferDirection
{
    Send,
    Receive,
}

/// <summary>Passed to <see cref="IWoooshCore.StartAsync"/> (DESIGN.md §4).</summary>
public sealed record CoreConfig
{
    public required string DisplayName { get; init; }

    public DeviceType DeviceType { get; init; } = DeviceType.Windows;

    public required CoreVisibility Visibility { get; init; }

    /// <summary>App-private directory where the core stages incoming files until FileReady.</summary>
    public required string StagingDir { get; init; }

    /// <summary>App-private JSON file holding the core's pinned peer keys (PROTOCOL.md §4.5).</summary>
    public required string TrustStorePath { get; init; }

    /// <summary>Bind address; null means 0.0.0.0 on an ephemeral UDP port.</summary>
    public string? ListenAddr { get; init; }
}

/// <summary>
/// A peer as the core sees it, keyed by long-term identity (DeviceID), not by the rotating
/// discovery id the mDNS browser uses.
///
/// <see cref="PublicKey"/> is the peer's raw 32-byte Ed25519 identity key, proven in the
/// TLS handshake. Pass it back to <see cref="IWoooshCore.ConnectPeerAsync"/> and
/// <see cref="IWoooshCore.RevokePeerAsync"/>: <see cref="Id"/> is a one-way BLAKE3
/// derivation and cannot be turned back into a pin (DESIGN.md §4).
/// </summary>
public sealed record PeerRef
{
    public required string Id { get; init; }

    public required string DisplayName { get; init; }

    /// <summary>Six-word phrase from the core's own wordlist. Never re-derived in the shell.</summary>
    public required string Fingerprint { get; init; }

    public required bool Paired { get; init; }

    public byte[]? PublicKey { get; init; }

    /// <summary>The peer's HELLO <c>dt</c>; Unknown when it announced a type this build does not know.</summary>
    public DeviceType DeviceType { get; init; } = DeviceType.Unknown;
}

/// <summary>
/// One pinned peer from the core's own trust store.
///
/// This is the shell's trust list. There is deliberately no local mirror of it: a
/// shell-side copy drifts from the core's <c>trust.json</c> the moment a pairing completes
/// on the far side of a QR (DESIGN.md §4).
/// </summary>
public sealed record TrustedPeerInfo
{
    public required string DeviceId { get; init; }

    /// <summary>Raw 32 bytes. What ConnectPeer pins against and RevokePeer takes.</summary>
    public required byte[] PublicKey { get; init; }

    public required string DisplayName { get; init; }

    public DeviceType DeviceType { get; init; } = DeviceType.Unknown;

    public required string Fingerprint { get; init; }

    public required DateTimeOffset PairedAt { get; init; }

    public required DateTimeOffset LastSeen { get; init; }
}

public sealed record FileMeta
{
    public required FileId Id { get; init; }

    public required string Name { get; init; }

    public required long Size { get; init; }

    public required string Mime { get; init; }
}

/// <summary>
/// A parsed <c>wooosh-pair:1?…</c> payload (PROTOCOL.md §4.2), read without dialling
/// anything. Everything here is unauthenticated hint material: it exists so the pairing UI
/// can name the peer while the blocking handshake runs, and can reject an expired or
/// malformed code instantly instead of after a network timeout.
/// </summary>
public sealed record PairingCodeInfo
{
    public required string DeviceId { get; init; }

    public string? DeviceName { get; init; }

    public required IReadOnlyList<string> Hints { get; init; }

    public required bool Expired { get; init; }
}

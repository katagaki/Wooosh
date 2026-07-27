using Wooosh.Peers;

namespace Wooosh.Core;

public readonly record struct TransferId(string Value)
{
    public override string ToString() => Value;
}

/// <summary><c>fid</c> on the wire (PROTOCOL.md §5), u32 in the core.</summary>
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

public sealed record CoreConfig
{
    public required string DisplayName { get; init; }

    public DeviceType DeviceType { get; init; } = DeviceType.Windows;

    public required CoreVisibility Visibility { get; init; }

    /// <summary>App-private staging for incoming files until FileReady.</summary>
    public required string StagingDir { get; init; }

    /// <summary>Pinned peer keys (PROTOCOL.md §4.5). App-private.</summary>
    public required string TrustStorePath { get; init; }

    /// <summary>Null means 0.0.0.0 on an ephemeral UDP port.</summary>
    public string? ListenAddr { get; init; }
}

/// <summary>Keyed by long-term identity, not the rotating mDNS discovery id.
/// <see cref="PublicKey"/> is the raw 32-byte Ed25519 key proven in the handshake and is what
/// pins; <see cref="Id"/> is a one-way BLAKE3 derivation of it (DESIGN.md §4).</summary>
public sealed record PeerRef
{
    public required string Id { get; init; }

    public required string DisplayName { get; init; }

    /// <summary>From the core's own wordlist. Never re-derived in the shell.</summary>
    public required string Fingerprint { get; init; }

    public required bool Paired { get; init; }

    public byte[]? PublicKey { get; init; }

    /// <summary>The peer's HELLO <c>dt</c>; Unknown when it announced a type this build does not know.</summary>
    public DeviceType DeviceType { get; init; } = DeviceType.Unknown;
}

/// <summary>Read live from the core's trust store, never mirrored: a shell-side copy drifts
/// the moment a pairing completes on the far side of a QR (DESIGN.md §4).</summary>
public sealed record TrustedPeerInfo
{
    public required string DeviceId { get; init; }

    /// <summary>Raw 32 bytes: what ConnectPeer pins against and RevokePeer takes.</summary>
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

/// <summary>A parsed <c>wooosh-pair:1?…</c> payload (PROTOCOL.md §4.2): unauthenticated hint
/// material for the pairing UI, never a basis for trust.</summary>
public sealed record PairingCodeInfo
{
    public required string DeviceId { get; init; }

    public string? DeviceName { get; init; }

    public required IReadOnlyList<string> Hints { get; init; }

    public required bool Expired { get; init; }
}

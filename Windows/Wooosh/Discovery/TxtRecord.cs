using Wooosh.Core;
using Wooosh.Peers;

namespace Wooosh.Discovery;

/// <summary>
/// The mDNS TXT record of PROTOCOL.md §3.1. Exactly six short keys, all UTF-8:
///
/// <list type="table">
/// <item><term>v</term><description>protocol version, always <c>1</c></description></item>
/// <item><term>rid</term><description>rotating discovery ID, 8 random bytes as lowercase hex</description></item>
/// <item><term>dn</term><description>display name</description></item>
/// <item><term>dt</term><description>device type; <c>windows</c> for this shell</description></item>
/// <item><term>p</term><description>QUIC UDP port</description></item>
/// <item><term>vis</term><description><c>e</c> (everyone) or <c>p</c> (paired only)</description></item>
/// </list>
///
/// The long-term public key is deliberately absent: identity is proven in the handshake,
/// and <c>rid</c> is what stops a passive listener tracking a device across networks.
/// Everything read out of a TXT record is untrusted UI hint material.
/// </summary>
public sealed record TxtRecord
{
    public const string ProtocolVersion = "1";

    public required string Rid { get; init; }

    public required string DisplayName { get; init; }

    public required DeviceType DeviceType { get; init; }

    public required int Port { get; init; }

    public required CoreVisibility Visibility { get; init; }

    /// <summary>
    /// Key/value pairs to publish. <c>dt</c> is omitted rather than guessed when the type
    /// is unknown, because an absent value reads as unknown on the far side and a wrong
    /// one renders a confidently wrong icon.
    /// </summary>
    public IReadOnlyDictionary<string, string> ToAttributes()
    {
        var attributes = new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["v"] = ProtocolVersion,
            ["rid"] = Rid,
            ["dn"] = DisplayName,
            ["p"] = Port.ToString(),
        };

        var deviceType = DeviceType.ToTxtValue();
        if (deviceType is not null)
        {
            attributes["dt"] = deviceType;
        }

        var visibility = Visibility.ToTxtValue();
        if (visibility is not null)
        {
            attributes["vis"] = visibility;
        }

        return attributes;
    }

    /// <summary>
    /// Parses the <c>System.Devices.Dnssd.TextAttributes</c> property, which arrives as
    /// "key=value" strings. Returns null when the record is not a usable Wooosh announce:
    /// a wrong protocol version, a missing <c>rid</c>, or a port that is not a port.
    /// </summary>
    public static TxtRecord? Parse(IEnumerable<string> textAttributes, string instanceName)
    {
        var map = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var attribute in textAttributes)
        {
            var separator = attribute.IndexOf('=');
            if (separator <= 0)
            {
                continue;
            }

            map[attribute[..separator]] = attribute[(separator + 1)..];
        }

        if (!map.TryGetValue("v", out var version) || version != ProtocolVersion)
        {
            return null;
        }

        if (!map.TryGetValue("rid", out var rid) || string.IsNullOrEmpty(rid))
        {
            return null;
        }

        if (!map.TryGetValue("p", out var portText) ||
            !int.TryParse(portText, out var port) ||
            port is <= 0 or > 65535)
        {
            return null;
        }

        return new TxtRecord
        {
            Rid = rid,
            // The instance name is the fallback label; a peer that advertises no dn still
            // needs something to show, and the instance name is what the OS resolved.
            DisplayName = map.TryGetValue("dn", out var name) && !string.IsNullOrWhiteSpace(name)
                ? name
                : instanceName,
            DeviceType = DeviceTypeExtensions.FromTxtValue(map.GetValueOrDefault("dt")),
            Port = port,
            Visibility = map.GetValueOrDefault("vis") switch
            {
                "p" => CoreVisibility.PairedOnly,
                _ => CoreVisibility.Everyone,
            },
        };
    }
}

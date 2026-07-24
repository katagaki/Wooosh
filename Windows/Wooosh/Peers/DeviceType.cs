namespace Wooosh.Peers;

/// <summary>
/// Advertised device type: the <c>dt</c> TXT value of PROTOCOL.md §3.1.
///
/// The vocabulary is deliberately platform-explicit rather than form-factor-only, because
/// form factor cannot tell a Pixel from an iPhone and the receiving UI has to pick a glyph.
/// A generic icon is always acceptable; a confidently wrong one is not.
/// </summary>
public enum DeviceType
{
    /// <summary>Absent <c>dt</c>, or a value this build does not know. Never guessed at.</summary>
    Unknown = 0,
    IPhone,
    IPad,
    Mac,
    Windows,
    AndroidPhone,
    AndroidTablet,
}

public static class DeviceTypeExtensions
{
    /// <summary>Wire value, or null for <see cref="DeviceType.Unknown"/> (omit the key).</summary>
    public static string? ToTxtValue(this DeviceType type) => type switch
    {
        DeviceType.IPhone => "iphone",
        DeviceType.IPad => "ipad",
        DeviceType.Mac => "mac",
        DeviceType.Windows => "windows",
        DeviceType.AndroidPhone => "android-phone",
        DeviceType.AndroidTablet => "android-tablet",
        _ => null,
    };

    /// <summary>
    /// Parses a <c>dt</c> TXT value. Unrecognised and absent values both become
    /// <see cref="DeviceType.Unknown"/>: PROTOCOL.md §3.1 requires tolerating additions
    /// to the enum rather than guessing at them.
    /// </summary>
    public static DeviceType FromTxtValue(string? value) => value switch
    {
        "iphone" => DeviceType.IPhone,
        "ipad" => DeviceType.IPad,
        "mac" => DeviceType.Mac,
        "windows" => DeviceType.Windows,
        "android-phone" => DeviceType.AndroidPhone,
        "android-tablet" => DeviceType.AndroidTablet,
        _ => DeviceType.Unknown,
    };

    /// <summary>
    /// Segoe Fluent Icons glyph for a device row.
    ///
    /// Deliberately coarse: the glyphs say "phone", "tablet", "computer" and never imply a
    /// platform, so an unknown <c>dt</c> can fall back without looking like a different
    /// kind of device. Platform is conveyed by the accessible name, not the picture.
    /// </summary>
    public static string ToGlyph(this DeviceType type) => type switch
    {
        DeviceType.IPhone or DeviceType.AndroidPhone => "\uE8EA", // CellPhone
        DeviceType.IPad or DeviceType.AndroidTablet => "\uE70A",  // Tablet
        DeviceType.Mac or DeviceType.Windows => "\uE7F8",         // DeviceLaptopNoPic
        _ => "\uE772",                                            // Devices: neutral, claims nothing
    };

    /// <summary>Resource key for the accessible name of the row's device glyph.</summary>
    public static string ToAccessibleNameKey(this DeviceType type) => type switch
    {
        DeviceType.IPhone => "DeviceKindIPhone",
        DeviceType.IPad => "DeviceKindIPad",
        DeviceType.Mac => "DeviceKindMac",
        DeviceType.Windows => "DeviceKindWindows",
        DeviceType.AndroidPhone => "DeviceKindAndroidPhone",
        DeviceType.AndroidTablet => "DeviceKindAndroidTablet",
        _ => "DeviceKindUnknown",
    };
}

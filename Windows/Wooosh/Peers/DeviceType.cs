namespace Wooosh.Peers;

/// <summary>PROTOCOL.md §3.1 <c>dt</c>: a generic icon is acceptable, a confidently wrong one is not.</summary>
public enum DeviceType
{
    /// <summary>Absent or unrecognised <c>dt</c>. Never guessed at.</summary>
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

    /// <summary>PROTOCOL.md §3.1 requires tolerating enum additions, not guessing at them.</summary>
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

    /// <summary>Coarse and never platform-implying, so Unknown can fall back; platform lives in the accessible name.</summary>
    public static string ToGlyph(this DeviceType type) => type switch
    {
        DeviceType.IPhone or DeviceType.AndroidPhone => "\uE8EA", // CellPhone
        DeviceType.IPad or DeviceType.AndroidTablet => "\uE70A",  // Tablet
        DeviceType.Mac or DeviceType.Windows => "\uE7F8",         // DeviceLaptopNoPic
        _ => "\uE772",                                            // Devices
    };

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

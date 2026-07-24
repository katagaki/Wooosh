using System.Globalization;

namespace Wooosh.Localization;

/// <summary>
/// Byte sizes and durations. One place, because COPY_STYLE.md §5 forbids hand-built size
/// and rate strings scattered through views.
/// </summary>
public static class Formatters
{
    /// <summary>
    /// Human-readable byte size, for example "12 MB".
    ///
    /// The number goes through the current culture's number formatter. The unit is the SI
    /// symbol, which is not translated in any of Wooosh's target locales. Neither .NET nor
    /// WinRT ships a localized byte-size formatter, so this is the substitute: if one
    /// appears, this method is the only thing that has to change.
    /// </summary>
    public static string ByteSize(long bytes)
    {
        string[] units = ["B", "kB", "MB", "GB", "TB"];
        double value = Math.Max(bytes, 0);
        var unit = 0;
        while (value >= 1000 && unit < units.Length - 1)
        {
            value /= 1000;
            unit++;
        }

        var decimals = unit == 0 ? 0 : value < 10 ? 1 : 0;
        return $"{value.ToString("N" + decimals, CultureInfo.CurrentCulture)} {units[unit]}";
    }

    /// <summary>Remaining time, "m:ss" or "h:mm:ss".</summary>
    public static string Duration(long seconds) =>
        TimeSpan.FromSeconds(Math.Max(seconds, 0))
            .ToString(seconds >= 3600 ? @"h\:mm\:ss" : @"m\:ss", CultureInfo.CurrentCulture);
}

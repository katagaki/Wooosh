using System.Globalization;

namespace Wooosh.Localization;

/// <summary>One place: COPY_STYLE.md §5 forbids hand-built size and rate strings in views.</summary>
public static class Formatters
{
    /// <summary>
    /// Neither .NET nor WinRT ships a localized byte-size formatter. The number goes
    /// through the current culture; the SI symbol is untranslated in every target locale.
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

    public static string Duration(long seconds) =>
        TimeSpan.FromSeconds(Math.Max(seconds, 0))
            .ToString(seconds >= 3600 ? @"h\:mm\:ss" : @"m\:ss", CultureInfo.CurrentCulture);
}

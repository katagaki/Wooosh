using System.Globalization;

namespace Wooosh.Localization;

/// <summary>
/// CLDR cardinal plural categories for the fourteen locales Wooosh ships
/// (COPY_STYLE.md §5, "Target locales").
///
/// Windows resource files have no plural machinery equivalent to Android's
/// &lt;plurals&gt; or an Apple String Catalog's plural variations, so the selection
/// happens here and the .resw carries one entry per form (Key.One, Key.Few, …).
/// The rules below are the integer-only subset of the CLDR cardinal rules: every count
/// Wooosh formats is a file count, never a fraction.
/// </summary>
public static class PluralRules
{
    /// <summary>Suffix appended to a resource key: One, Few, Many, or Other.</summary>
    public static string CategoryFor(CultureInfo culture, int count)
    {
        var n = Math.Abs(count);
        return culture.TwoLetterISOLanguageName switch
        {
            // No grammatical plural at all.
            "ja" or "ko" or "zh" => "Other",

            // one: i = 0..1 (so "0 fichier", singular).
            "fr" or "pt" => n is 0 or 1 ? "One" : "Other",

            // one: n = 1.
            "en" or "de" or "es" or "it" or "nl" or "sv" => n == 1 ? "One" : "Other",

            // one: n = 1; few: n % 10 = 2..4 and n % 100 != 12..14; many: everything else.
            "pl" => n switch
            {
                1 => "One",
                _ when n % 10 is >= 2 and <= 4 && n % 100 is < 12 or > 14 => "Few",
                _ => "Many",
            },

            // one: n % 10 = 1 and n % 100 != 11; few / many as Polish.
            "ru" => n switch
            {
                _ when n % 10 == 1 && n % 100 != 11 => "One",
                _ when n % 10 is >= 2 and <= 4 && n % 100 is < 12 or > 14 => "Few",
                _ => "Many",
            },

            // A language Wooosh does not ship yet: Other always exists.
            _ => "Other",
        };
    }
}

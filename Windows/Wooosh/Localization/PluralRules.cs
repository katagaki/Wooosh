using System.Globalization;

namespace Wooosh.Localization;

/// <summary>
/// CLDR cardinal categories for every locale Wooosh ships (COPY_STYLE.md §5). Windows
/// .resw has no equivalent of Android &lt;plurals&gt; or Apple's variations, so selection
/// happens here and the .resw carries one entry per form. Integers only: these are counts.
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

            // i = 0..1, so "0 fichier" is singular.
            "fr" or "pt" => n is 0 or 1 ? "One" : "Other",

            "en" or "de" or "es" or "it" or "nl" or "sv" => n == 1 ? "One" : "Other",

            "pl" => n switch
            {
                1 => "One",
                _ when n % 10 is >= 2 and <= 4 && n % 100 is < 12 or > 14 => "Few",
                _ => "Many",
            },

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

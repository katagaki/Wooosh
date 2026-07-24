using System.Globalization;
using Microsoft.Windows.ApplicationModel.Resources;

namespace Wooosh.Localization;

/// <summary>
/// The only way display text enters the UI (COPY_STYLE.md §5). Nothing in Views/ or
/// ViewModels/ may hold a literal user-facing string.
///
/// Backed by Strings/&lt;locale&gt;/Resources.resw through MRT Core, which resolves the
/// locale itself from the user's Windows language list.
/// </summary>
public static class Strings
{
    private static readonly ResourceLoader Loader = new();

    /// <summary>Localized text for <paramref name="key"/>.</summary>
    /// <remarks>
    /// A missing key returns the key itself rather than an empty string: a visibly wrong
    /// label in a screenshot is findable, a blank one silently ships.
    /// </remarks>
    public static string Get(string key)
    {
        var value = Loader.GetString(key);
        return string.IsNullOrEmpty(value) ? key : value;
    }

    /// <summary>
    /// Localized format string filled with <paramref name="args"/>.
    /// The .resw values use positional placeholders ({0}, {1}) so translators can reorder
    /// freely; never build a sentence by concatenating fragments.
    /// </summary>
    public static string Format(string key, params object[] args) =>
        string.Format(CultureInfo.CurrentCulture, Get(key), args);

    /// <summary>
    /// Localized text for a countable quantity.
    ///
    /// Picks <paramref name="key"/>.One / .Few / .Many / .Other by the CLDR plural
    /// category of the current UI language (see <see cref="PluralRules"/>), because
    /// Polish and Russian have four forms and an English-shaped <c>count == 1</c> test
    /// is wrong in both.
    /// </summary>
    public static string Plural(string key, int count, params object[] args)
    {
        var category = PluralRules.CategoryFor(CultureInfo.CurrentUICulture, count);
        var value = Loader.GetString($"{key}.{category}");
        if (string.IsNullOrEmpty(value))
        {
            // Every locale defines Other; the narrower forms are optional per CLDR.
            value = Loader.GetString($"{key}.Other");
        }

        return string.IsNullOrEmpty(value)
            ? key
            : string.Format(CultureInfo.CurrentCulture, value, args);
    }
}

using System.Globalization;
using Microsoft.Windows.ApplicationModel.Resources;

namespace Wooosh.Localization;

/// <summary>The only way display text enters the UI: no literal user-facing string elsewhere (COPY_STYLE.md §5).</summary>
public static class Strings
{
    private static readonly ResourceLoader Loader = new();

    /// <summary>A missing key returns the key: a wrong label is findable, a blank one ships.</summary>
    public static string Get(string key)
    {
        var value = Loader.GetString(key);
        return string.IsNullOrEmpty(value) ? key : value;
    }

    /// <summary>Positional placeholders so translators can reorder; never concatenate fragments.</summary>
    public static string Format(string key, params object[] args) =>
        string.Format(CultureInfo.CurrentCulture, Get(key), args);

    /// <summary>CLDR categories: Polish and Russian have four forms, so <c>count == 1</c> is wrong in both.</summary>
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

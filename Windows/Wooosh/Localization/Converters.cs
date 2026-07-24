using Microsoft.UI.Xaml.Data;

namespace Wooosh.Localization;

/// <summary>
/// Resource key to localized string, for values a view model produces as a key rather than
/// as text (the device-kind accessible names, for instance).
/// </summary>
public sealed partial class LocalizeConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language) =>
        value is string key ? Strings.Get(key) : string.Empty;

    public object ConvertBack(object value, Type targetType, object parameter, string language) =>
        throw new NotSupportedException();
}

/// <summary>
/// True (stale) to a dimmed opacity. The row is dimmed, not removed and not moved
/// (DESIGN.md §5).
/// </summary>
public sealed partial class StaleOpacityConverter : IValueConverter
{
    public double StaleOpacity { get; set; } = 0.4;

    public object Convert(object value, Type targetType, object parameter, string language) =>
        value is true ? StaleOpacity : 1.0;

    public object ConvertBack(object value, Type targetType, object parameter, string language) =>
        throw new NotSupportedException();
}

/// <summary>True to <c>Visibility.Visible</c>.</summary>
public sealed partial class BoolToVisibilityConverter : IValueConverter
{
    public bool Invert { get; set; }

    public object Convert(object value, Type targetType, object parameter, string language) =>
        value is true != Invert
            ? Microsoft.UI.Xaml.Visibility.Visible
            : Microsoft.UI.Xaml.Visibility.Collapsed;

    public object ConvertBack(object value, Type targetType, object parameter, string language) =>
        throw new NotSupportedException();
}

/// <summary>Non-empty string to <c>Visibility.Visible</c>, for optional banners.</summary>
public sealed partial class StringToVisibilityConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, string language) =>
        value is string { Length: > 0 }
            ? Microsoft.UI.Xaml.Visibility.Visible
            : Microsoft.UI.Xaml.Visibility.Collapsed;

    public object ConvertBack(object value, Type targetType, object parameter, string language) =>
        throw new NotSupportedException();
}

using Microsoft.UI.Xaml.Markup;

namespace Wooosh.Localization;

/// <summary>
/// Preferred over <c>x:Uid</c>, which forces keys to be named after the XAML element and
/// property and so cannot share one key name per concept with the Android and Apple shells.
/// </summary>
[MarkupExtensionReturnType(ReturnType = typeof(string))]
public sealed partial class LocalizeExtension : MarkupExtension
{
    public string Key { get; set; } = string.Empty;

    protected override object ProvideValue() => Strings.Get(Key);
}

using Microsoft.UI.Xaml.Markup;

namespace Wooosh.Localization;

/// <summary>
/// XAML markup extension that pulls static text out of Strings/&lt;locale&gt;/Resources.resw:
/// <c>Text="{loc:Localize Key=EmptyTitle}"</c>.
///
/// Preferred over <c>x:Uid</c> here for one reason: <c>x:Uid</c> forces resource keys to
/// be named after the XAML element and the property it sets (<c>EmptyTitleText.Text</c>),
/// which makes it impossible to keep the key names aligned with the Android
/// <c>strings.xml</c> and Apple String Catalog they were translated from. One key name per
/// concept across all three shells is worth a nine-line markup extension.
/// </summary>
[MarkupExtensionReturnType(ReturnType = typeof(string))]
public sealed partial class LocalizeExtension : MarkupExtension
{
    public string Key { get; set; } = string.Empty;

    protected override object ProvideValue() => Strings.Get(Key);
}

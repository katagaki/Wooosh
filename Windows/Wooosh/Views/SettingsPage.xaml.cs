using System.ComponentModel;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wooosh.Core;
using Wooosh.Localization;
using Wooosh.ViewModels;

namespace Wooosh.Views;

public sealed partial class SettingsPage : Page, INotifyPropertyChanged
{
    public SettingsPage()
    {
        InitializeComponent();

        if (App.ViewModel is not { } viewModel)
        {
            return;
        }

        var settings = viewModel.Settings.Current;
        DisplayNameBox.Text = settings.DisplayName;
        KeepRunningToggle.IsOn = settings.KeepRunningInBackground;

        switch (settings.Visibility)
        {
            case CoreVisibility.PairedOnly:
                VisibilityPaired.IsChecked = true;
                break;
            case CoreVisibility.Off:
                VisibilityOff.IsChecked = true;
                break;
            default:
                VisibilityEveryone.IsChecked = true;
                break;
        }

        // The core is started asynchronously, so the identity fields usually arrive after
        // this page is first drawn.
        viewModel.PropertyChanged += OnViewModelPropertyChanged;
        Unloaded += (_, _) => viewModel.PropertyChanged -= OnViewModelPropertyChanged;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    private void OnViewModelPropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName is nameof(MainViewModel.DeviceId))
        {
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(DeviceIdText)));
        }
        else if (e.PropertyName is nameof(MainViewModel.FingerprintPhrase))
        {
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(FingerprintText)));
        }
    }

    /// <summary>The core's own DeviceID, or a placeholder while it is still starting.</summary>
    public string DeviceIdText => App.ViewModel?.DeviceId ?? Strings.Get("SettingsStarting");

    /// <summary>
    /// This device's six-word phrase, as the core derives it. Fingerprint phrases are never
    /// translated: they are a shared artifact that must read identically on both devices.
    /// </summary>
    public string FingerprintText => App.ViewModel?.FingerprintPhrase ?? Strings.Get("SettingsStarting");

    private void OnBackClick(object sender, RoutedEventArgs e)
    {
        if (Frame.CanGoBack)
        {
            Frame.GoBack();
        }
    }

    /// <summary>
    /// Committed on focus loss, not per keystroke. Every change republishes the mDNS record,
    /// and doing that on each character makes this device flicker in every other device's
    /// list (the advertiser debounces as well, belt and braces).
    /// </summary>
    private void OnDisplayNameCommitted(object sender, RoutedEventArgs e) =>
        App.ViewModel?.Settings.SetDisplayName(DisplayNameBox.Text);

    private void OnVisibilityChanged(object sender, RoutedEventArgs e)
    {
        if (sender is not RadioButton { Tag: string tag })
        {
            return;
        }

        App.ViewModel?.SetVisibility(tag switch
        {
            "pairedOnly" => CoreVisibility.PairedOnly,
            "off" => CoreVisibility.Off,
            _ => CoreVisibility.Everyone,
        });
    }

    private void OnKeepRunningToggled(object sender, RoutedEventArgs e) =>
        App.ViewModel?.Settings.SetKeepRunningInBackground(KeepRunningToggle.IsOn);
}

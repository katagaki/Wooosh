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

        // The core starts asynchronously, so the identity fields arrive after this page is first drawn.
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

    public string DeviceIdText => App.ViewModel?.DeviceId ?? Strings.Get("SettingsStarting");

    /// <summary>Never translated: the phrase must read identically on both devices.</summary>
    public string FingerprintText => App.ViewModel?.FingerprintPhrase ?? Strings.Get("SettingsStarting");

    private void OnBackClick(object sender, RoutedEventArgs e)
    {
        if (Frame.CanGoBack)
        {
            Frame.GoBack();
        }
    }

    /// <summary>Focus loss, not per keystroke: every change republishes the mDNS record and would flicker this device in every other list.</summary>
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

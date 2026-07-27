using System.ComponentModel;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wooosh.Peers;
using Wooosh.ViewModels;

namespace Wooosh.Views;

/// <summary>Renders <see cref="PeerRegistry"/> verbatim: first-discovery order, never re-sorted, departed devices dimmed in place.</summary>
public sealed partial class DeviceListPage : Page, INotifyPropertyChanged
{
    public DeviceListPage()
    {
        InitializeComponent();

        if (ViewModel is { } viewModel)
        {
            viewModel.Peers.CollectionChanged += (_, _) => Raise(nameof(EmptyStateVisibility));
        }
    }

    public MainViewModel? ViewModel => App.ViewModel;

    /// <summary>Rows are never removed once seen, so this flips exactly once per session.</summary>
    public Visibility EmptyStateVisibility =>
        ViewModel is null || ViewModel.Peers.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

    public event PropertyChangedEventHandler? PropertyChanged;

    private void Raise(string name) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));

    private void OnRefreshClick(object sender, RoutedEventArgs e) => ViewModel?.Refresh();

    private void OnPairClick(object sender, RoutedEventArgs e) => Frame.Navigate(typeof(PairingPage));

    private void OnSettingsClick(object sender, RoutedEventArgs e) => Frame.Navigate(typeof(SettingsPage));

    private void OnOtherDeviceClick(object sender, RoutedEventArgs e) => Frame.Navigate(typeof(OtherDevicePage));

    private void OnPeerClick(object sender, ItemClickEventArgs e)
    {
        if (e.ClickedItem is not Peer peer || peer.IsStale)
        {
            return;
        }

        // TODO(send): picker, then connect_peer(address, pinned key) and send(); both block and must run off the UI thread.
        System.Diagnostics.Debug.WriteLine($"[Wooosh] send to {peer.Label} at {peer.Address ?? "(unresolved)"}");
    }
}

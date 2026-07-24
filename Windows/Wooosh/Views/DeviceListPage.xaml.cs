using System.ComponentModel;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wooosh.Peers;
using Wooosh.ViewModels;

namespace Wooosh.Views;

/// <summary>
/// The device list. The one screen whose rules are non-negotiable: rows are in
/// first-discovery order, they are never re-sorted, and a device that goes away dims in
/// place rather than disappearing. Those rules live in <see cref="PeerRegistry"/> and this
/// page renders them verbatim.
/// </summary>
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

    /// <summary>
    /// The empty state is only for a genuinely empty list. Once a device has been seen it
    /// keeps its row forever, stale or not, so this flips exactly once per session.
    /// </summary>
    public Visibility EmptyStateVisibility =>
        ViewModel is null || ViewModel.Peers.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

    public event PropertyChangedEventHandler? PropertyChanged;

    private void Raise(string name) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));

    private void OnRefreshClick(object sender, RoutedEventArgs e) => ViewModel?.Refresh();

    private void OnPairClick(object sender, RoutedEventArgs e) => Frame.Navigate(typeof(PairingPage));

    private void OnSettingsClick(object sender, RoutedEventArgs e) => Frame.Navigate(typeof(SettingsPage));

    private void OnPeerClick(object sender, ItemClickEventArgs e)
    {
        if (e.ClickedItem is not Peer peer || peer.IsStale)
        {
            return;
        }

        // TODO(send): open the file picker, then connect_peer(peer.Address, pinned key from
        // trustedPeers()) followed by send(). Both calls block and must run off the UI
        // thread. Blocked on the core bindings; the row itself is real and clickable today.
        System.Diagnostics.Debug.WriteLine($"[Wooosh] send to {peer.Label} at {peer.Address ?? "(unresolved)"}");
    }
}

using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage;
using Windows.Storage.Pickers;
using Wooosh.Core;
using Wooosh.Localization;

namespace Wooosh.Views;

/// <summary>
/// Transfers with a device that is not on this network (PROTOCOL.md §9).
///
/// One screen, two segments. The sender publishes a code and waits; the receiver redeems
/// one. <b>Nothing is paired.</b> The code authorises a single transfer and dies with it,
/// so no verification phrase is shown on either side: there is no prior relationship to
/// check one against, and a verification step the user cannot actually perform is worse
/// than none.
/// </summary>
public sealed partial class OtherDevicePage : Page
{
    private IReadOnlyList<string> _stagedPaths = [];
    private bool _hasTicket;

    /// <summary>
    /// The SelectorBar raises SelectionChanged while the XAML tree is still being built,
    /// before the panels it wants to show and hide have been assigned to their fields.
    /// </summary>
    private readonly bool _initialized;

    public OtherDevicePage()
    {
        InitializeComponent();
        _initialized = true;
    }

    private static IWoooshCore? Core => App.ViewModel?.Core;

    /// <summary>
    /// A live code is a capability, so leaving the screen revokes it rather than letting
    /// it linger until its own expiry.
    /// </summary>
    protected override void OnNavigatedFrom(Microsoft.UI.Xaml.Navigation.NavigationEventArgs e)
    {
        base.OnNavigatedFrom(e);
        EndTicket();
    }

    private void EndTicket()
    {
        if (!_hasTicket)
        {
            return;
        }

        _hasTicket = false;
        try
        {
            Core?.EndInternetTicket();
        }
        catch (CoreException)
        {
            // Revoking a code on the way out has nothing left to tell the user: the
            // screen is already gone.
        }
    }

    private void OnBackClick(object sender, RoutedEventArgs e)
    {
        if (Frame.CanGoBack)
        {
            Frame.GoBack();
        }
    }

    /// <summary>
    /// Switching segments tears the other direction's state down, which is also what ends
    /// a code that is on screen. Deliberate: a published code must not outlive the panel
    /// showing it.
    /// </summary>
    private void OnDirectionChanged(SelectorBar sender, SelectorBarSelectionChangedEventArgs args)
    {
        if (!_initialized)
        {
            return;
        }

        var sending = sender.SelectedItem == SendSegment;

        SendPanel.Visibility = sending ? Visibility.Visible : Visibility.Collapsed;
        ReceivePanel.Visibility = sending ? Visibility.Collapsed : Visibility.Visible;

        if (!sending)
        {
            ResetSend();
        }

        Status.IsOpen = false;
    }

    private void ResetSend()
    {
        EndTicket();
        _stagedPaths = [];
        TicketBox.Text = string.Empty;
        CopyTicketButton.Content = Strings.Get("ActionCopyCode");
        ShowSendStage(SendIntro);
    }

    private void ShowSendStage(FrameworkElement stage)
    {
        foreach (var panel in new FrameworkElement[] { SendIntro, SendMinting, SendTicket, SendStarted })
        {
            panel.Visibility = panel == stage ? Visibility.Visible : Visibility.Collapsed;
        }
    }

    // ---- send ----

    private async void OnChooseFilesClick(object sender, RoutedEventArgs e)
    {
        var picker = new FileOpenPicker { FileTypeFilter = { "*" } };
        // WinUI 3 pickers are window-owned and throw without an owner HWND.
        WinRT.Interop.InitializeWithWindow.Initialize(picker, App.MainWindowHandle);

        IReadOnlyList<StorageFile> files = await picker.PickMultipleFilesAsync();
        if (files.Count == 0)
        {
            return;
        }

        // Paths straight through: the core reads them where they are and derives name and
        // MIME itself. Nothing is renamed or transcoded on the way out.
        _stagedPaths = files.Select(f => f.Path).ToList();

        Status.IsOpen = false;
        ShowSendStage(SendMinting);

        try
        {
            var ticket = await Core!.BeginInternetTicketAsync();
            _hasTicket = true;
            TicketBox.Text = ticket;
            ShowSendStage(SendTicket);

            // TODO(send): the ticket is live but nothing hands the files over yet. That
            // needs the TicketRedeemed core event, which needs the CoreEventListener
            // callback codec, and then SendAsync(peerId, _stagedPaths). Until both land
            // this screen publishes a code that no transfer follows, which is why the
            // waiting state never resolves.
            System.Diagnostics.Debug.WriteLine(
                $"[Wooosh] internet ticket published for {_stagedPaths.Count} file(s); redemption not wired.");
        }
        catch (Exception error)
        {
            ShowSendStage(SendIntro);
            ShowStatus(error is CoreException ? error.Message : Strings.Get("ErrorTicketFailed"));
        }
    }

    private void OnCopyTicketClick(object sender, RoutedEventArgs e)
    {
        if (TicketBox.Text.Length == 0)
        {
            return;
        }

        var package = new DataPackage();
        package.SetText(TicketBox.Text);
        Clipboard.SetContent(package);
        CopyTicketButton.Content = Strings.Get("ActionCopied");
    }

    // ---- receive ----

    private void OnPasteBoxChanged(object sender, TextChangedEventArgs e) =>
        RedeemButton.IsEnabled = PasteBox.Text.Trim().Length > 0;

    private async void OnRedeemClick(object sender, RoutedEventArgs e)
    {
        var ticket = PasteBox.Text.Trim();
        if (ticket.Length == 0)
        {
            return;
        }

        RedeemButton.IsEnabled = false;
        // Hole punching before the reply is slower than a connection on the same network,
        // and silence for that long reads as a hang.
        ShowStatus(Strings.Get("OtherDeviceConnectingBody"), InfoBarSeverity.Informational);

        try
        {
            await Core!.RedeemTicketAsync(ticket);
            // Redeeming is the whole job: the sender hands the files over on its own and
            // the incoming transfer shows in the device list like any other.
            Status.IsOpen = false;
            if (Frame.CanGoBack)
            {
                Frame.GoBack();
            }
        }
        catch (Exception error)
        {
            ShowStatus(error is CoreException ? error.Message : Strings.Get("ErrorTicketFailed"));
        }
        finally
        {
            RedeemButton.IsEnabled = PasteBox.Text.Trim().Length > 0;
        }
    }

    private void ShowStatus(string message, InfoBarSeverity severity = InfoBarSeverity.Error)
    {
        Status.Message = message;
        Status.Severity = severity;
        Status.IsOpen = true;
    }
}

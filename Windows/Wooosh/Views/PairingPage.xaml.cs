using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.ApplicationModel.DataTransfer;
using Wooosh.Core;
using Wooosh.Localization;

namespace Wooosh.Views;

/// <summary>QR pairing (PROTOCOL.md §4.2) from the desktop side, where a pasted code stands in for scanning.</summary>
public sealed partial class PairingPage : Page
{
    public PairingPage()
    {
        InitializeComponent();
        Loaded += (_, _) => RefreshCode();
    }

    private void RefreshCode()
    {
        if (App.ViewModel is not { } viewModel)
        {
            return;
        }

        try
        {
            // The core mints the payload; the shell never builds a pairing URL itself.
            MyCodeBox.Text = viewModel.Core.BeginPairingQr();
        }
        catch (CoreException e)
        {
            MyCodeBox.Text = string.Empty;
            ShowStatus(e.Message, InfoBarSeverity.Error);
        }
    }

    private void OnBackClick(object sender, RoutedEventArgs e)
    {
        if (Frame.CanGoBack)
        {
            Frame.GoBack();
        }
    }

    private void OnCopyCodeClick(object sender, RoutedEventArgs e)
    {
        if (MyCodeBox.Text.Length == 0)
        {
            return;
        }

        var package = new DataPackage();
        package.SetText(MyCodeBox.Text);
        Clipboard.SetContent(package);
        CopyCodeButton.Content = Strings.Get("ActionCopied");
    }

    private void OnNewCodeClick(object sender, RoutedEventArgs e)
    {
        CopyCodeButton.Content = Strings.Get("ActionCopyCode");
        RefreshCode();
    }

    private void OnPairWithPastedClick(object sender, RoutedEventArgs e)
    {
        var payload = PasteBox.Text.Trim();
        if (payload.Length == 0)
        {
            return;
        }

        // TODO(pairing): parse locally first, then run PairWithQrAsync off the UI thread and drive this page off the PairingResult event.
        ShowStatus(Strings.Get("ErrorNotStarted"), InfoBarSeverity.Informational);
    }

    private void ShowStatus(string message, InfoBarSeverity severity)
    {
        PairingStatus.Message = message;
        PairingStatus.Severity = severity;
        PairingStatus.IsOpen = true;
    }
}

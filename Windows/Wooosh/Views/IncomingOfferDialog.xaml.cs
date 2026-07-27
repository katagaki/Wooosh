using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wooosh.Core;
using Wooosh.Localization;

namespace Wooosh.Views;

/// <summary>An unpaired sender must show its verification phrase, and "Accept Once" must let the user take files without pinning (PROTOCOL.md §4.4).</summary>
public sealed partial class IncomingOfferDialog : ContentDialog
{
    private readonly CoreEvent.IncomingOffer _offer;

    public IncomingOfferDialog(CoreEvent.IncomingOffer offer)
    {
        InitializeComponent();
        _offer = offer;

        var fileCount = offer.Manifest.Count;
        var totalBytes = offer.Manifest.Sum(file => file.Size);

        OfferHeadline.Text = Strings.Plural("OfferTitle", fileCount, offer.From.DisplayName, fileCount);
        OfferSummary.Text = Strings.Plural("FilesCountAndSize", fileCount, fileCount, Formatters.ByteSize(totalBytes));

        if (offer.From.Paired)
        {
            PairedLabel.Visibility = Visibility.Visible;
            PrimaryButtonText = Strings.Get("ActionAccept");
        }
        else
        {
            UnpairedBlock.Visibility = Visibility.Visible;
            FingerprintText.Text = offer.From.Fingerprint;
            // "Accept Once" never writes the trust store (PROTOCOL.md §4.4).
            PrimaryButtonText = Strings.Get("ActionAcceptOnce");
        }
    }

    /// <summary>Empty means the whole offer was declined.</summary>
    public IReadOnlyList<FileId> AcceptedFileIds { get; private set; } = [];

    private void OnPrimaryButtonClick(ContentDialog sender, ContentDialogButtonClickEventArgs args) =>
        AcceptedFileIds = [.. _offer.Manifest.Select(file => file.Id)];
}

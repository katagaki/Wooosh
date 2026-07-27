using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wooosh.ViewModels;

namespace Wooosh.Views;

public sealed partial class TransferProgressControl : UserControl
{
    public static readonly DependencyProperty TransferProperty = DependencyProperty.Register(
        nameof(Transfer),
        typeof(TransferViewModel),
        typeof(TransferProgressControl),
        new PropertyMetadata(null));

    public TransferProgressControl()
    {
        InitializeComponent();
    }

    public TransferViewModel? Transfer
    {
        get => (TransferViewModel?)GetValue(TransferProperty);
        set => SetValue(TransferProperty, value);
    }

    private void OnCancelClick(object sender, RoutedEventArgs e)
    {
        if (Transfer is null)
        {
            return;
        }

        App.ViewModel?.Core.Cancel(Transfer.Id);
    }
}

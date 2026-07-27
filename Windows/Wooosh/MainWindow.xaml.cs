using Microsoft.UI.Xaml;
using Wooosh.Platform;
using Wooosh.Views;
using WinRT.Interop;

namespace Wooosh;

public sealed partial class MainWindow : Window
{
    private TrayIcon? _tray;

    public MainWindow()
    {
        InitializeComponent();

        Title = "Wooosh";
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(TitleBarArea);

        RootFrame.Navigate(typeof(DeviceListPage));

        var hwnd = WindowNative.GetWindowHandle(this);
        _tray = new TrayIcon(hwnd);
        _tray.OpenRequested += ShowWindow;
        _tray.QuitRequested += QuitForReal;

        Closed += OnClosed;
        AppWindow.Closing += OnAppWindowClosing;
    }

    /// <summary>Closing hides, so Wooosh keeps receiving from the tray (DESIGN.md §7).</summary>
    private void OnAppWindowClosing(
        Microsoft.UI.Windowing.AppWindow sender,
        Microsoft.UI.Windowing.AppWindowClosingEventArgs args)
    {
        if (App.ViewModel?.Settings.Current.KeepRunningInBackground != true)
        {
            return;
        }

        args.Cancel = true;
        AppWindow.Hide();
    }

    private void ShowWindow()
    {
        AppWindow.Show();
        Activate();
    }

    private void QuitForReal()
    {
        AppWindow.Closing -= OnAppWindowClosing;
        Close();
    }

    private async void OnClosed(object sender, WindowEventArgs args)
    {
        _tray?.Dispose();
        _tray = null;

        if (App.ViewModel is { } viewModel)
        {
            // Stop blocks for roughly 2 s of runtime shutdown, so it is awaited, not fired off.
            await viewModel.DisposeAsync();
        }
    }
}

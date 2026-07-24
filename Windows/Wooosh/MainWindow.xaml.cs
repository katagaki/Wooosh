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

    /// <summary>
    /// Closing the window hides it instead of exiting, so Wooosh keeps receiving from the
    /// notification area (DESIGN.md §7). The user can still quit from the tray menu, and
    /// the setting turns the behaviour off.
    /// </summary>
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
            // Stop is blocking (roughly 2 s of runtime shutdown), which is exactly why it
            // is awaited here rather than run on the way out of a synchronous handler.
            await viewModel.DisposeAsync();
        }
    }
}

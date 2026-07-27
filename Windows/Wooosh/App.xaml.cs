using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using Wooosh.Core;
using Wooosh.Settings;
using Wooosh.ViewModels;

namespace Wooosh;

public partial class App : Application
{
    /// <summary>On the application, not the window: the core and peer list outlive a minimise to tray.</summary>
    public static MainViewModel? ViewModel { get; private set; }

    /// <summary>WinUI 3 pickers are window-owned and throw without an HWND.</summary>
    public static IntPtr MainWindowHandle { get; private set; }

    private MainWindow? _window;

    public App()
    {
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        _window = new MainWindow();
        MainWindowHandle = WinRT.Interop.WindowNative.GetWindowHandle(_window);

        var settings = new SettingsRepository();
        var core = new NativeWoooshCore();
        ViewModel = new MainViewModel(_window.DispatcherQueue, core, settings);

        _window.Activate();

        // Fire and forget: failures surface through StartupError, and awaiting only delays the window.
        _ = ViewModel.StartAsync();

        HandleActivation();
    }

    /// <summary>Share Target entry point (DESIGN.md §8); Windows reuses the running instance.</summary>
    private void HandleActivation()
    {
        var activation = AppInstance.GetCurrent().GetActivatedEventArgs();
        if (activation.Kind != ExtendedActivationKind.ShareTarget)
        {
            return;
        }

        // TODO(share): stage the ShareOperation's items, then show the picker. Report complete
        // on staging, not on transfer end, or Windows blocks the source app for the whole send.
        System.Diagnostics.Debug.WriteLine("[Wooosh] activated as a share target; not handled yet.");
    }
}

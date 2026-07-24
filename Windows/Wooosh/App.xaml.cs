using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using Wooosh.Core;
using Wooosh.Settings;
using Wooosh.ViewModels;

namespace Wooosh;

public partial class App : Application
{
    /// <summary>
    /// The single view model for the process. Held on the application, not the window,
    /// because the window is closed and reopened when Wooosh minimises to the notification
    /// area and the core, discovery and peer list must survive that.
    /// </summary>
    public static MainViewModel? ViewModel { get; private set; }

    private MainWindow? _window;

    public App()
    {
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        _window = new MainWindow();

        var settings = new SettingsRepository();
        var core = new NativeWoooshCore();
        ViewModel = new MainViewModel(_window.DispatcherQueue, core, settings);

        _window.Activate();

        // Fire and forget by design: StartAsync does its blocking work on a background
        // thread and reports failure through MainViewModel.StartupError, which the device
        // list renders. Awaiting here would only delay the window.
        _ = ViewModel.StartAsync();

        HandleActivation();
    }

    /// <summary>
    /// Share Target entry point (DESIGN.md §8). Windows activates the already-running
    /// instance, so this is also reached on a second launch from the share sheet.
    /// </summary>
    private void HandleActivation()
    {
        var activation = AppInstance.GetCurrent().GetActivatedEventArgs();
        if (activation.Kind != ExtendedActivationKind.ShareTarget)
        {
            return;
        }

        // TODO(share): pull the StorageItems out of
        // ((ShareTargetActivatedEventArgs)activation.Data).ShareOperation, copy them into the
        // app's local staging folder, and open the device list in "pick a device to send to"
        // mode with the same ordering and staleness rules as the main list. The share
        // operation must be reported complete once the files are staged, not once the
        // transfer finishes, or Windows keeps the source app blocked for the whole send.
        System.Diagnostics.Debug.WriteLine("[Wooosh] activated as a share target; not handled yet.");
    }
}

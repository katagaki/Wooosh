using System.Runtime.InteropServices;
using Wooosh.Localization;

namespace Wooosh.Platform;

/// <summary>
/// Notification-area icon so Wooosh can keep receiving with its window closed
/// (DESIGN.md §7: "minimizes to tray and keeps receiving").
///
/// <para>WinUI 3 has no tray API, so this is <c>Shell_NotifyIcon</c> directly. The
/// alternative, a NuGet wrapper, adds a dependency for about a hundred lines of P/Invoke
/// that will not change again.</para>
///
/// <para>The callback message is delivered to the window procedure of an existing HWND, so
/// this subclasses the main window rather than creating a message-only window: one fewer
/// window to keep alive, and the icon dies with the window it belongs to.</para>
/// </summary>
public sealed partial class TrayIcon : IDisposable
{
    private const int WmApp = 0x8000;
    private const int WmTrayCallback = WmApp + 1;
    private const int WmCommand = 0x0111;
    private const int WmLButtonUp = 0x0202;
    private const int WmRButtonUp = 0x0205;

    private const int IdOpen = 1;
    private const int IdQuit = 2;

    private readonly IntPtr _hwnd;
    private readonly IntPtr _previousWndProc;
    private readonly WndProc _wndProc;
    private NotifyIconData _data;
    private bool _disposed;

    /// <summary>Raised when the user asks for the window back.</summary>
    public event Action? OpenRequested;

    /// <summary>Raised when the user quits from the menu. This is a real exit, not a hide.</summary>
    public event Action? QuitRequested;

    public TrayIcon(IntPtr hwnd)
    {
        _hwnd = hwnd;

        // Keep the delegate alive for as long as the subclass is installed: if it is
        // collected, the next message dispatched to it takes the process down.
        _wndProc = HandleMessage;
        _previousWndProc = SetWindowProc(hwnd, Marshal.GetFunctionPointerForDelegate(_wndProc));

        _data = new NotifyIconData
        {
            Size = (uint)Marshal.SizeOf<NotifyIconData>(),
            Window = hwnd,
            Id = 1,
            Flags = NifMessage | NifIcon | NifTip,
            CallbackMessage = WmTrayCallback,
            Icon = LoadIconW(IntPtr.Zero, IdiApplication),
            // The product name is never translated (COPY_STYLE.md §2).
            Tip = "Wooosh",
        };

        Shell_NotifyIconW(NimAdd, ref _data);
    }

    private IntPtr HandleMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam)
    {
        switch (message)
        {
            case WmTrayCallback when (int)lParam == WmLButtonUp:
                OpenRequested?.Invoke();
                return IntPtr.Zero;

            case WmTrayCallback when (int)lParam == WmRButtonUp:
                ShowMenu();
                return IntPtr.Zero;

            case WmCommand when (int)wParam == IdOpen:
                OpenRequested?.Invoke();
                return IntPtr.Zero;

            case WmCommand when (int)wParam == IdQuit:
                QuitRequested?.Invoke();
                return IntPtr.Zero;

            default:
                return CallWindowProcW(_previousWndProc, hwnd, message, wParam, lParam);
        }
    }

    private void ShowMenu()
    {
        var menu = CreatePopupMenu();
        if (menu == IntPtr.Zero)
        {
            return;
        }

        try
        {
            AppendMenuW(menu, MfString, IdOpen, Strings.Get("TrayOpen"));
            AppendMenuW(menu, MfSeparator, 0, null);
            AppendMenuW(menu, MfString, IdQuit, Strings.Get("TrayQuit"));

            // Required, or the menu stays up after a click elsewhere (documented quirk of
            // TrackPopupMenu from a notification-icon callback).
            SetForegroundWindow(_hwnd);
            GetCursorPos(out var cursor);
            TrackPopupMenuEx(menu, TpmRightButton, cursor.X, cursor.Y, _hwnd, IntPtr.Zero);
            PostMessageW(_hwnd, 0, IntPtr.Zero, IntPtr.Zero);
        }
        finally
        {
            DestroyMenu(menu);
        }
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }

        _disposed = true;
        Shell_NotifyIconW(NimDelete, ref _data);
        SetWindowProc(_hwnd, _previousWndProc);
    }

    // ---- Win32 ------------------------------------------------------------------------

    private const int GwlpWndProc = -4;

    private const uint NimAdd = 0x00000000;
    private const uint NimDelete = 0x00000002;
    private const uint NifMessage = 0x00000001;
    private const uint NifIcon = 0x00000002;
    private const uint NifTip = 0x00000004;

    private const uint MfString = 0x00000000;
    private const uint MfSeparator = 0x00000800;
    private const uint TpmRightButton = 0x0002;

    private static readonly IntPtr IdiApplication = new(32512);

    private delegate IntPtr WndProc(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NotifyIconData
    {
        public uint Size;
        public IntPtr Window;
        public uint Id;
        public uint Flags;
        public uint CallbackMessage;
        public IntPtr Icon;

        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string Tip;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Point
    {
        public int X;
        public int Y;
    }

    // NOTIFYICONDATAW contains a fixed-size string, which the LibraryImport source
    // generator cannot marshal, so these stay on DllImport.
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool Shell_NotifyIconW(uint message, ref NotifyIconData data);

    /// <summary>
    /// SetWindowLongPtrW only exists on 64-bit user32; on x86 it is a macro over
    /// SetWindowLongW. The project builds for x86 as well, so both are declared.
    /// </summary>
    private static IntPtr SetWindowProc(IntPtr hwnd, IntPtr wndProc) =>
        IntPtr.Size == 8
            ? SetWindowLongPtrW(hwnd, GwlpWndProc, wndProc)
            : new IntPtr(SetWindowLongW(hwnd, GwlpWndProc, wndProc.ToInt32()));

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr SetWindowLongPtrW(IntPtr hwnd, int index, IntPtr newLong);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern int SetWindowLongW(IntPtr hwnd, int index, int newLong);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr CallWindowProcW(IntPtr previous, IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr LoadIconW(IntPtr instance, IntPtr name);

    [DllImport("user32.dll")]
    private static extern IntPtr CreatePopupMenu();

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool AppendMenuW(IntPtr menu, uint flags, int id, string? item);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DestroyMenu(IntPtr menu);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool TrackPopupMenuEx(IntPtr menu, uint flags, int x, int y, IntPtr hwnd, IntPtr parameters);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetForegroundWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetCursorPos(out Point point);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
}

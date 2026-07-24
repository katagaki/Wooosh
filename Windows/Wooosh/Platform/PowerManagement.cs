using System.Runtime.InteropServices;

namespace Wooosh.Platform;

/// <summary>
/// Keeps the machine awake while a transfer is running (DESIGN.md §7).
///
/// <c>ES_CONTINUOUS | ES_SYSTEM_REQUIRED</c> and nothing else: the system stays up so the
/// transfer completes, but the display is free to sleep. A desktop file transfer has no
/// business holding the screen on, and <c>ES_DISPLAY_REQUIRED</c> on a laptop closing its
/// lid overnight is a flat battery.
///
/// The flags are per-thread state, so the acquire and the release must happen on the same
/// thread. <see cref="Acquire"/> is therefore expected to be called from the UI thread on
/// TransferStarted and released on TransferDone, and it refcounts so several concurrent
/// transfers do not release each other's hold.
/// </summary>
public static partial class PowerManagement
{
    private const uint EsContinuous = 0x80000000;
    private const uint EsSystemRequired = 0x00000001;

    private static int _holds;

    /// <summary>
    /// Takes a hold. Dispose the returned token to release it; the machine is only allowed
    /// to sleep again once every hold is gone.
    /// </summary>
    public static IDisposable Acquire()
    {
        if (Interlocked.Increment(ref _holds) == 1)
        {
            SetThreadExecutionState(EsContinuous | EsSystemRequired);
        }

        return new Hold();
    }

    private static void Release()
    {
        if (Interlocked.Decrement(ref _holds) == 0)
        {
            // ES_CONTINUOUS on its own clears the requirement without asserting a new one.
            SetThreadExecutionState(EsContinuous);
        }
    }

    private sealed class Hold : IDisposable
    {
        private bool _released;

        public void Dispose()
        {
            if (_released)
            {
                return;
            }

            _released = true;
            Release();
        }
    }

    [LibraryImport("kernel32.dll")]
    private static partial uint SetThreadExecutionState(uint flags);
}

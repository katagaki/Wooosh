using System.Runtime.InteropServices;

namespace Wooosh.Platform;

/// <summary>
/// Keeps the machine awake for a transfer (DESIGN.md §7). System only, no
/// <c>ES_DISPLAY_REQUIRED</c>: holding a laptop's screen on overnight is a flat battery.
/// Per-thread state, so acquire and release must share a thread; holds are refcounted.
/// </summary>
public static partial class PowerManagement
{
    private const uint EsContinuous = 0x80000000;
    private const uint EsSystemRequired = 0x00000001;

    private static int _holds;

    /// <summary>The machine may sleep again only once every hold is disposed.</summary>
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

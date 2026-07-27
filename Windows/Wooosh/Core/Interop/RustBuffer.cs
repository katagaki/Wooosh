using System.Runtime.InteropServices;
using System.Text;

namespace Wooosh.Core.Interop;

/// <summary><c>struct RustBuffer { uint64_t capacity; uint64_t len; uint8_t *data; }</c>.
/// Field order, widths and sequential layout are load-bearing: a mismatch is silent memory
/// corruption, not a compile error.</summary>
[StructLayout(LayoutKind.Sequential)]
internal struct RustBuffer
{
    public ulong Capacity;
    public ulong Len;
    public IntPtr Data;

    public static RustBuffer Empty => new() { Capacity = 0, Len = 0, Data = IntPtr.Zero };

    public readonly byte[] ToBytes()
    {
        if (Data == IntPtr.Zero || Len == 0)
        {
            return [];
        }

        var bytes = new byte[Len];
        Marshal.Copy(Data, bytes, 0, checked((int)Len));
        return bytes;
    }

    /// <summary>Everything crossing the FFI must come from the Rust allocator: the two sides
    /// do not share a heap.</summary>
    public static RustBuffer FromBytes(byte[] bytes)
    {
        var pinned = GCHandle.Alloc(bytes, GCHandleType.Pinned);
        try
        {
            var foreign = new ForeignBytes
            {
                Len = bytes.Length,
                Data = bytes.Length == 0 ? IntPtr.Zero : pinned.AddrOfPinnedObject(),
            };
            return UniffiCall.Rust((ref RustCallStatus status) =>
                NativeMethods.ffi_wooosh_core_rustbuffer_from_bytes(foreign, ref status));
        }
        finally
        {
            pinned.Free();
        }
    }

    public void Free()
    {
        if (Data == IntPtr.Zero && Capacity == 0)
        {
            return;
        }

        var self = this;
        UniffiCall.Rust((ref RustCallStatus status) =>
            NativeMethods.ffi_wooosh_core_rustbuffer_free(self, ref status));
        Data = IntPtr.Zero;
        Len = 0;
        Capacity = 0;
    }
}

/// <summary>A borrowed view of memory the foreign side owns.</summary>
[StructLayout(LayoutKind.Sequential)]
internal struct ForeignBytes
{
    public int Len;
    public IntPtr Data;
}

/// <summary>0 on success, 1 when <see cref="ErrorBuf"/> holds a lowered error, 2 a panic.</summary>
[StructLayout(LayoutKind.Sequential)]
internal struct RustCallStatus
{
    public sbyte Code;
    public RustBuffer ErrorBuf;

    public const sbyte Success = 0;
    public const sbyte Error = 1;
    public const sbyte Panic = 2;
}

/// <summary>Every scaffolding call goes through here so a Rust error or panic becomes a C# exception.</summary>
internal static class UniffiCall
{
    internal delegate T RustCallback<out T>(ref RustCallStatus status);

    internal delegate void RustVoidCallback(ref RustCallStatus status);

    public static T Rust<T>(RustCallback<T> call)
    {
        var status = default(RustCallStatus);
        var result = call(ref status);
        Check(ref status);
        return result;
    }

    public static void Rust(RustVoidCallback call)
    {
        var status = default(RustCallStatus);
        call(ref status);
        Check(ref status);
    }

    private static void Check(ref RustCallStatus status)
    {
        switch (status.Code)
        {
            case RustCallStatus.Success:
                return;

            case RustCallStatus.Error:
                // TODO(bindings): the payload is a lowered CoreError enum needing the
                // generated reader, so the variant is lost. CoreErrors.cs must key off the
                // decoded variant, never off English text.
                status.ErrorBuf.Free();
                throw new CoreException(Localization.Strings.Get("ErrorTransferFailed"));

            case RustCallStatus.Panic:
                var message = Encoding.UTF8.GetString(status.ErrorBuf.ToBytes());
                status.ErrorBuf.Free();
                throw new InvalidOperationException($"wooosh-core panicked: {message}");

            default:
                throw new InvalidOperationException(
                    $"wooosh-core returned an unknown RustCallStatus code {status.Code}.");
        }
    }
}

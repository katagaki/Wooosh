using System.Runtime.InteropServices;
using System.Text;

namespace Wooosh.Platform;

/// <summary>
/// Hash-verified files leave staging for Downloads, photos included (DESIGN.md §6). Never
/// overwrite: collisions append " (2)", because silent replacement is unrecoverable. Never
/// otherwise rename. Always mark zone 3, or the user gets an unmarked foreign executable.
/// </summary>
public static partial class StorageRouter
{
    public static string RouteToDownloads(string stagedPath, string originalName)
    {
        var destination = NextAvailablePath(DownloadsPath(), originalName);

        // TODO(DESIGN.md §6): >20-file receives belong in a Wooosh/<date> subfolder, which needs a count this call lacks.
        File.Move(stagedPath, destination, overwrite: false);
        ApplyMarkOfTheWeb(destination);
        return destination;
    }

    /// <summary>Neither <c>KnownFolders</c> nor <c>SpecialFolder</c> exposes Downloads, and <c>DownloadsFolder</c> renames.</summary>
    private static string DownloadsPath()
    {
        var hr = NativeMethods.SHGetKnownFolderPath(
            NativeMethods.FolderIdDownloads, dwFlags: 0, hToken: IntPtr.Zero, out var buffer);
        if (hr != 0 || buffer == IntPtr.Zero)
        {
            throw new IOException($"Could not locate the Downloads folder (HRESULT 0x{hr:X8}).");
        }

        try
        {
            return Marshal.PtrToStringUni(buffer)!;
        }
        finally
        {
            Marshal.FreeCoTaskMem(buffer);
        }
    }

    /// <summary>The suffix goes before the extension, so the file still opens with the right application.</summary>
    private static string NextAvailablePath(string directory, string originalName)
    {
        var stem = Path.GetFileNameWithoutExtension(originalName);
        var extension = Path.GetExtension(originalName);

        var candidate = Path.Combine(directory, originalName);
        for (var index = 2; File.Exists(candidate) || Directory.Exists(candidate); index++)
        {
            candidate = Path.Combine(directory, $"{stem} ({index}){extension}");
        }

        return candidate;
    }

    /// <summary>Zone 3 is URLZONE_INTERNET. Win32, because .NET rejects the <c>path:stream</c> syntax.</summary>
    public static void ApplyMarkOfTheWeb(string path)
    {
        const string content =
            "[ZoneTransfer]\r\n" +
            "ZoneId=3\r\n" +
            // No host name: naming the sender would leak it into every file's metadata.
            "HostUrl=about:internet\r\n";

        var stream = NativeMethods.CreateFileW(
            $"{path}:Zone.Identifier",
            NativeMethods.GenericWrite,
            shareMode: 0,
            securityAttributes: IntPtr.Zero,
            creationDisposition: NativeMethods.CreateAlways,
            flagsAndAttributes: NativeMethods.FileAttributeNormal,
            templateFile: IntPtr.Zero);

        if (stream == NativeMethods.InvalidHandle)
        {
            // Not fatal: the file is already in place, and failing the transfer would be worse.
            System.Diagnostics.Debug.WriteLine(
                $"[Wooosh] could not write Zone.Identifier for {path}: {Marshal.GetLastWin32Error()}");
            return;
        }

        try
        {
            var bytes = Encoding.ASCII.GetBytes(content);
            NativeMethods.WriteFile(stream, bytes, (uint)bytes.Length, out _, IntPtr.Zero);
        }
        finally
        {
            NativeMethods.CloseHandle(stream);
        }
    }

    private static partial class NativeMethods
    {
        public const uint GenericWrite = 0x40000000;
        public const uint CreateAlways = 2;
        public const uint FileAttributeNormal = 0x80;
        public static readonly IntPtr InvalidHandle = new(-1);

        // FOLDERID_Downloads.
        public static readonly Guid FolderIdDownloads =
            new("374DE290-123F-4565-9164-39C4925E467B");

        [LibraryImport("shell32.dll")]
        public static partial int SHGetKnownFolderPath(
            in Guid rfid,
            uint dwFlags,
            IntPtr hToken,
            out IntPtr ppszPath);

        [LibraryImport("kernel32.dll", EntryPoint = "CreateFileW", SetLastError = true,
            StringMarshalling = StringMarshalling.Utf16)]
        public static partial IntPtr CreateFileW(
            string fileName,
            uint desiredAccess,
            uint shareMode,
            IntPtr securityAttributes,
            uint creationDisposition,
            uint flagsAndAttributes,
            IntPtr templateFile);

        [LibraryImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static partial bool WriteFile(
            IntPtr file,
            byte[] buffer,
            uint bytesToWrite,
            out uint bytesWritten,
            IntPtr overlapped);

        [LibraryImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static partial bool CloseHandle(IntPtr handle);
    }
}

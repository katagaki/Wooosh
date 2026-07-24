using System.Runtime.InteropServices;
using System.Text;
using Windows.Storage;

namespace Wooosh.Platform;

/// <summary>
/// Moves a verified file out of the core's staging directory into its final place
/// (DESIGN.md §6). On Windows every received file goes to Downloads, photos included.
///
/// <para>Three rules, all of them load-bearing:</para>
/// <list type="bullet">
/// <item><b>Never overwrite.</b> A name collision appends " (2)", " (3)" and so on, before
/// the extension. A received file silently replacing one the user already had is data
/// loss, and it is not recoverable.</item>
/// <item><b>Never rename otherwise.</b> The original filename is preserved exactly,
/// including its extension and case.</item>
/// <item><b>Mark of the Web.</b> Anything that arrived over the network gets a
/// <c>Zone.Identifier</c> alternate data stream marking it zone 3 (internet), the same as a
/// browser download. That is what makes SmartScreen warn, Office open in Protected View,
/// and unblocking a deliberate step. A file transfer app that strips this is handing the
/// user an unmarked executable from another machine.</item>
/// </list>
/// </summary>
public static partial class StorageRouter
{
    /// <summary>
    /// Moves <paramref name="stagedPath"/> into Downloads and returns the final path.
    /// The move is what makes the transfer complete: nothing is reported as received until
    /// the file is where the user can find it.
    /// </summary>
    public static async Task<string> RouteToDownloadsAsync(string stagedPath, string originalName)
    {
        // KnownFolders.DownloadsFolder is the documented Windows destination (DESIGN.md §6).
        // A packaged app gets write access to it without a broadFileSystemAccess capability.
        var downloads = KnownFolders.DownloadsFolder;
        var destination = await NextAvailablePathAsync(downloads, originalName);

        // TODO(DESIGN.md §6): when a single receive brings more than 20 files, land them in
        // a Wooosh/<date> subfolder of Downloads instead of the root. That needs the file
        // count for the whole transfer, which lives in TransferStarted, so it belongs to the
        // transfer coordinator rather than to this per-file call.
        File.Move(stagedPath, destination, overwrite: false);
        ApplyMarkOfTheWeb(destination);
        return destination;
    }

    /// <summary>
    /// "photo.jpg" then "photo (2).jpg" then "photo (3).jpg". The suffix goes before the
    /// extension so the file still opens with the right application.
    /// </summary>
    private static async Task<string> NextAvailablePathAsync(StorageFolder folder, string originalName)
    {
        var directory = folder.Path;
        var stem = Path.GetFileNameWithoutExtension(originalName);
        var extension = Path.GetExtension(originalName);

        var candidate = Path.Combine(directory, originalName);
        for (var index = 2; File.Exists(candidate) || Directory.Exists(candidate); index++)
        {
            candidate = Path.Combine(directory, $"{stem} ({index}){extension}");
        }

        // Touching the folder through the WinRT API first surfaces a denied Downloads
        // folder as a clean failure here rather than as an IOException mid-move.
        _ = await folder.GetBasicPropertiesAsync();
        return candidate;
    }

    /// <summary>
    /// Writes the <c>Zone.Identifier</c> alternate data stream. Zone 3 is URLZONE_INTERNET.
    ///
    /// The ADS is written with the Win32 API rather than <c>File.WriteAllText</c>, because
    /// .NET's file APIs reject the <c>path:stream</c> syntax.
    /// </summary>
    public static void ApplyMarkOfTheWeb(string path)
    {
        const string content =
            "[ZoneTransfer]\r\n" +
            "ZoneId=3\r\n" +
            // Recorded for the shell UI's "unblock" prompt. No host name is disclosed:
            // naming the sending device here would leak it into every file's metadata.
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
            // Not fatal: the file itself is already safely in place, and failing the whole
            // transfer over a missing zone marker would be worse than the missing marker.
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

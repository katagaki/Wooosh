using System.Security.Cryptography;
using Windows.Storage;

namespace Wooosh.Platform;

/// <summary>
/// The Windows <c>KeyStore</c> adapter (DESIGN.md §4, PROTOCOL.md §2). The Ed25519
/// identity is DPAPI-wrapped and scoped to the current user, so another account on the
/// machine cannot read it and a copied profile cannot carry it. The core calls in
/// synchronously and DPAPI can block, so <c>start</c> must never run on the UI thread.
/// Not yet wired to the core: the UniFFI callback bindings do not exist.
/// </summary>
public sealed class DpapiKeyStore
{
    private const string FileName = "identity.key";

    /// <summary>Not a secret: it scopes the blob so another app running as this user cannot unprotect it.</summary>
    private static readonly byte[] Entropy = "com.tsubuzaki.Wooosh/identity/v1"u8.ToArray();

    private readonly string _path;

    public DpapiKeyStore()
    {
        _path = Path.Combine(ApplicationData.Current.LocalFolder.Path, FileName);
    }

    public byte[]? LoadIdentity()
    {
        if (!File.Exists(_path))
        {
            return null;
        }

        var protectedBytes = File.ReadAllBytes(_path);
        try
        {
            return ProtectedData.Unprotect(protectedBytes, Entropy, DataProtectionScope.CurrentUser);
        }
        catch (CryptographicException)
        {
            // Undecryptable blob (copied profile, restored backup): null makes the core mint a
            // fresh identity, so peers hard-fail on KEY_CHANGED as PROTOCOL.md §4.5 requires.
            return null;
        }
    }

    public void StoreIdentity(byte[] identity)
    {
        var protectedBytes = ProtectedData.Protect(identity, Entropy, DataProtectionScope.CurrentUser);

        // Temp-and-swap: a crash mid-write must not leave a truncated, different identity.
        var temporary = _path + ".tmp";
        File.WriteAllBytes(temporary, protectedBytes);
        File.Move(temporary, _path, overwrite: true);
    }
}

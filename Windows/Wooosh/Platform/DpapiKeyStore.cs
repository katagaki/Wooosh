using System.Security.Cryptography;
using Windows.Storage;

namespace Wooosh.Platform;

/// <summary>
/// The Windows half of the core's <c>KeyStore</c> platform adapter (DESIGN.md §4,
/// PROTOCOL.md §2): the Ed25519 identity key is persisted DPAPI-wrapped, scoped to the
/// current user, so it is unreadable by another account on the same machine and does not
/// travel with a copied profile.
///
/// <para><b>Blocking on purpose.</b> The core calls <c>load_identity</c> and
/// <c>store_identity</c> synchronously on whatever thread called <c>start</c>. DPAPI can
/// block, so <c>start</c> must never run on the UI thread. That is the contract, not an
/// implementation detail (DESIGN.md §4, threading).</para>
///
/// <para><b>Not yet connected.</b> The core reaches this class through a UniFFI callback
/// interface, and that VTable is part of the generated bindings that do not exist yet. The
/// storage itself below is complete and can be tested on its own.</para>
/// </summary>
public sealed class DpapiKeyStore
{
    private const string FileName = "identity.key";

    /// <summary>
    /// Entropy mixed into the DPAPI blob. Not a secret and not a substitute for one: it
    /// scopes the blob to Wooosh so another application running as the same user cannot
    /// unprotect it by accident.
    /// </summary>
    private static readonly byte[] Entropy = "com.tsubuzaki.Wooosh/identity/v1"u8.ToArray();

    private readonly string _path;

    public DpapiKeyStore()
    {
        _path = Path.Combine(ApplicationData.Current.LocalFolder.Path, FileName);
    }

    /// <summary>The stored identity key, or null on first launch.</summary>
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
            // The blob exists but this user cannot decrypt it: a copied profile, or a
            // restored backup from another machine. Returning null makes the core mint a
            // fresh identity, which every peer will correctly see as a KEY_CHANGED and
            // refuse until re-paired. That is the intended outcome; silently re-pinning is
            // exactly what PROTOCOL.md §4.5 forbids.
            return null;
        }
    }

    public void StoreIdentity(byte[] identity)
    {
        var protectedBytes = ProtectedData.Protect(identity, Entropy, DataProtectionScope.CurrentUser);

        // Write to a temporary file and swap, so a crash mid-write cannot leave a truncated
        // identity that reads back as a different device.
        var temporary = _path + ".tmp";
        File.WriteAllBytes(temporary, protectedBytes);
        File.Move(temporary, _path, overwrite: true);
    }
}

using System.Runtime.InteropServices;

namespace Wooosh.Core.Interop;

/// <summary>
/// P/Invoke declarations for the wooosh-core UniFFI scaffolding ABI.
///
/// <para><b>Why P/Invoke and not generated bindings.</b> UniFFI ships first-party
/// generators for Kotlin, Swift and Python only. The C# generator
/// (<c>uniffi-bindgen-cs</c>, NordSecurity) exists and is the right long-term answer, but
/// it tracks UniFFI versions with a lag and the core is on UniFFI 0.29 today. Rather than
/// pin the whole Windows shell to whether that generator has caught up, this file declares
/// the scaffolding ABI by hand. The symbol names and signatures below were read directly
/// out of the generated <c>Core/bindings/swift/wooosh_coreFFI.h</c> in this repository, so
/// they are the real contract, not a guess.</para>
///
/// <para><b>What is finished and what is not.</b> The ABI surface here is complete and the
/// primitive lowering in <see cref="UniffiSerialization"/> is real. What is missing is the
/// generated codec for the core's own records, enums and callback interfaces
/// (<c>Config</c>, <c>PeerRef</c>, <c>CoreEvent</c>, <c>TrustedPeer</c>, <c>KeyStore</c>,
/// <c>CoreEventListener</c>). Those are byte-for-byte serialisation formats that must
/// match the Rust side exactly, and writing them by hand from memory is how you get a
/// shell that reads a peer's public key out of the wrong offset. See
/// <c>Windows/README.md</c> for the two supported ways to fill that gap.</para>
///
/// <para>The DLL must be built for the same architecture as the app
/// (<c>cargo build --release --target x86_64-pc-windows-msvc</c> and friends) and sit next
/// to <c>Wooosh.exe</c>.</para>
/// </summary>
internal static partial class NativeMethods
{
    /// <summary>
    /// Matches <c>[lib] name = "wooosh_core"</c> in <c>Core/wooosh-core/Cargo.toml</c>, so
    /// the cdylib is <c>wooosh_core.dll</c> on Windows.
    /// </summary>
    private const string Library = "wooosh_core";

    /// <summary>
    /// UniFFI bindings and scaffolding must agree on this or every later call is
    /// undefined. 29 is UniFFI 0.29, which is what <c>Core/wooosh-core/Cargo.toml</c>
    /// depends on and what the generated Swift bindings assert.
    /// </summary>
    public const uint ExpectedContractVersion = 29;

    // ---- runtime support ------------------------------------------------------------

    [LibraryImport(Library)]
    public static partial uint ffi_wooosh_core_uniffi_contract_version();

    [LibraryImport(Library)]
    public static partial RustBuffer ffi_wooosh_core_rustbuffer_alloc(ulong size, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer ffi_wooosh_core_rustbuffer_from_bytes(ForeignBytes bytes, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial void ffi_wooosh_core_rustbuffer_free(RustBuffer buffer, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer ffi_wooosh_core_rustbuffer_reserve(RustBuffer buffer, ulong additional, ref RustCallStatus status);

    // ---- WoooshCore object ----------------------------------------------------------

    [LibraryImport(Library)]
    public static partial IntPtr uniffi_wooosh_core_fn_constructor_woooshcore_new(ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial IntPtr uniffi_wooosh_core_fn_clone_woooshcore(IntPtr ptr, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_free_woooshcore(IntPtr ptr, ref RustCallStatus status);

    /// <param name="config">Lowered <c>Config</c> record.</param>
    /// <param name="keyStore">Handle to a foreign <c>KeyStore</c> callback object.</param>
    /// <param name="listener">Handle to a foreign <c>CoreEventListener</c> callback object.</param>
    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_method_woooshcore_start(
        IntPtr ptr, RustBuffer config, IntPtr keyStore, IntPtr listener, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_method_woooshcore_stop(IntPtr ptr, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_method_woooshcore_set_visibility(
        IntPtr ptr, RustBuffer mode, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_method_woooshcore_device_id(IntPtr ptr, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_method_woooshcore_fingerprint_phrase(IntPtr ptr, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_method_woooshcore_public_key(IntPtr ptr, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_method_woooshcore_listen_addr(IntPtr ptr, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_method_woooshcore_trusted_peers(IntPtr ptr, ref RustCallStatus status);

    /// <returns>i8 used as a bool: 1 when the key was pinned and has now been dropped.</returns>
    [LibraryImport(Library)]
    public static partial sbyte uniffi_wooosh_core_fn_method_woooshcore_revoke_peer(
        IntPtr ptr, RustBuffer pubkey, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_method_woooshcore_begin_pairing_qr(IntPtr ptr, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_method_woooshcore_pair_with_qr(
        IntPtr ptr, RustBuffer payload, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_method_woooshcore_request_sas_pairing(
        IntPtr ptr, RustBuffer peerId, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_method_woooshcore_confirm_sas(
        IntPtr ptr, RustBuffer peerId, sbyte accepted, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_method_woooshcore_connect_peer(
        IntPtr ptr, RustBuffer addr, RustBuffer expectedPubkey, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_method_woooshcore_send(
        IntPtr ptr, RustBuffer peerId, RustBuffer files, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_method_woooshcore_respond_to_offer(
        IntPtr ptr, RustBuffer transferId, RustBuffer acceptedFileIds, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_method_woooshcore_resume_transfer(
        IntPtr ptr, RustBuffer peerId, RustBuffer transferId, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_method_woooshcore_cancel(
        IntPtr ptr, RustBuffer transferId, RustBuffer fileId, ref RustCallStatus status);

    // ---- free functions -------------------------------------------------------------

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_func_device_id_for(RustBuffer pubkey, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_func_fingerprint_phrase_for(RustBuffer pubkey, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_func_parse_pairing_qr(RustBuffer payload, ref RustCallStatus status);

    // ---- callback interfaces --------------------------------------------------------
    //
    // TODO(bindings): KeyStore and CoreEventListener are foreign-implemented traits. Each
    // needs a VTable struct of function pointers registered once at startup via the
    // init_callback_vtable entry point below, plus a handle map keeping the managed
    // objects alive for as long as Rust holds a handle. Getting the VTable field order
    // wrong is an immediate crash inside the core, which is why these are declared but
    // not yet called.

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_init_callback_vtable_keystore(IntPtr vtable);

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_init_callback_vtable_coreeventlistener(IntPtr vtable);

    /// <summary>
    /// The core also ships a <c>FileKeyStore</c> that persists the identity key to a path.
    /// On Windows that file must additionally be wrapped with DPAPI (PROTOCOL.md §2), so
    /// the shell supplies its own <c>KeyStore</c> instead. See Platform/DpapiKeyStore.cs.
    /// </summary>
    [LibraryImport(Library)]
    public static partial IntPtr uniffi_wooosh_core_fn_constructor_filekeystore_new(
        RustBuffer path, ref RustCallStatus status);
}

using System.Runtime.InteropServices;

namespace Wooosh.Core.Interop;

/// <summary>The wooosh-core UniFFI scaffolding ABI, transcribed from the generated
/// <c>Core/bindings/swift/wooosh_coreFFI.h</c> because UniFFI has no first-party C#
/// generator. The DLL must match the app's architecture and sit next to
/// <c>Wooosh.exe</c>.</summary>
internal static partial class NativeMethods
{
    private const string Library = "wooosh_core";

    /// <summary>Bindings and scaffolding must agree or every later call is undefined.</summary>
    public const uint ExpectedContractVersion = 29;

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

    [LibraryImport(Library)]
    public static partial IntPtr uniffi_wooosh_core_fn_constructor_woooshcore_new(ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial IntPtr uniffi_wooosh_core_fn_clone_woooshcore(IntPtr ptr, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_free_woooshcore(IntPtr ptr, ref RustCallStatus status);

    /// <summary><c>keyStore</c> and <c>listener</c> are handles to foreign callback objects.</summary>
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

    /// <returns>i8 as a bool: 1 when the key was pinned and has now been dropped.</returns>
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

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_func_device_id_for(RustBuffer pubkey, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_func_fingerprint_phrase_for(RustBuffer pubkey, ref RustCallStatus status);

    [LibraryImport(Library)]
    public static partial RustBuffer uniffi_wooosh_core_fn_func_parse_pairing_qr(RustBuffer payload, ref RustCallStatus status);

    // TODO(bindings): each needs a VTable of function pointers registered at startup plus a
    // handle map keeping the managed objects alive while Rust holds a handle. A wrong field
    // order crashes inside the core, so these are declared but not called.

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_init_callback_vtable_keystore(IntPtr vtable);

    [LibraryImport(Library)]
    public static partial void uniffi_wooosh_core_fn_init_callback_vtable_coreeventlistener(IntPtr vtable);

    /// <summary>Unused: the identity key must be DPAPI-wrapped on Windows (PROTOCOL.md §2).</summary>
    [LibraryImport(Library)]
    public static partial IntPtr uniffi_wooosh_core_fn_constructor_filekeystore_new(
        RustBuffer path, ref RustCallStatus status);
}

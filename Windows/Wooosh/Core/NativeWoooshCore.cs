using Wooosh.Core.Interop;
using Wooosh.Localization;

namespace Wooosh.Core;

/// <summary>
/// The real core, over <c>wooosh_core.dll</c>. The only implementation of
/// <see cref="IWoooshCore"/> there will ever be: the Apple and Android shells both had a
/// mock removed, and a shell that can fabricate peers and transfers makes every demo,
/// screenshot and bug report ambiguous.
///
/// <para><b>State of this file.</b> Three groups of members:</para>
/// <list type="number">
/// <item><b>Working.</b> Everything whose arguments and results are strings, byte arrays or
/// booleans. Those lower and lift through <see cref="UniffiSerialization"/>, which is the
/// complete UniFFI primitive format, so they will work the moment the DLL is present.</item>
/// <item><b>Blocked on a record codec.</b> <see cref="StartAsync"/>,
/// <see cref="TrustedPeersAsync"/>, <see cref="SendAsync"/>, <see cref="ParsePairingCode"/>
/// and <see cref="SetVisibility"/>. These pass or return the core's own record and enum
/// types, whose byte layout must match the Rust declarations exactly.</item>
/// <item><b>Blocked on callback VTables.</b> The event stream and the key store. Rust calls
/// back into managed code through a registered table of function pointers; the order of
/// that table is generated, not documented.</item>
/// </list>
///
/// <para>Groups 2 and 3 throw <see cref="CoreException"/> with a user-presentable message
/// rather than returning something invented. See Windows/README.md for how to close
/// the gap.</para>
/// </summary>
public sealed class NativeWoooshCore : IWoooshCore
{
    private readonly object _gate = new();
    private IntPtr _handle = IntPtr.Zero;

    public event Action<CoreEvent>? EventReceived;

    public string? DeviceId { get; private set; }

    public string? FingerprintPhrase { get; private set; }

    public string? ListenAddr { get; private set; }

    /// <summary>
    /// True when <c>wooosh_core.dll</c> loads and reports the UniFFI contract version these
    /// declarations were written against. Checked before anything else so a missing or
    /// mismatched DLL is one clear failure at startup rather than a crash later.
    /// </summary>
    public static bool ProbeNativeLibrary(out string diagnostic)
    {
        try
        {
            var version = NativeMethods.ffi_wooosh_core_uniffi_contract_version();
            if (version != NativeMethods.ExpectedContractVersion)
            {
                diagnostic =
                    $"wooosh_core.dll reports UniFFI contract version {version}, " +
                    $"these bindings were written for {NativeMethods.ExpectedContractVersion}.";
                return false;
            }

            diagnostic = string.Empty;
            return true;
        }
        catch (DllNotFoundException)
        {
            diagnostic = "wooosh_core.dll was not found next to Wooosh.exe.";
            return false;
        }
        catch (EntryPointNotFoundException e)
        {
            diagnostic = $"wooosh_core.dll is missing a UniFFI entry point: {e.Message}";
            return false;
        }
    }

    public Task StartAsync(CoreConfig config, CancellationToken cancellationToken = default) =>
        // Off the UI thread, always: start() runs the whole boot inline and calls
        // KeyStore.load_identity synchronously, which on Windows means DPAPI and possibly
        // a Windows Hello prompt (DESIGN.md §4, threading contract).
        Task.Run(
            () =>
            {
                if (!ProbeNativeLibrary(out var diagnostic))
                {
                    throw new CoreException(Strings.Get("ErrorCoreStart"), new InvalidOperationException(diagnostic));
                }

                lock (_gate)
                {
                    _handle = UniffiCall.Rust(
                        (ref RustCallStatus status) =>
                            NativeMethods.uniffi_wooosh_core_fn_constructor_woooshcore_new(ref status));
                }

                // TODO(bindings): lower `config` (a Config record), build the KeyStore and
                // CoreEventListener callback objects, then:
                //
                //   NativeMethods.uniffi_wooosh_core_fn_method_woooshcore_start(
                //       _handle, loweredConfig, keyStoreHandle, listenerHandle, ref status);
                //
                // then read back device_id / fingerprint_phrase / listen_addr, which are
                // plain strings and already work. Until the callback VTables exist there is
                // nothing to hand start() for its last two arguments, and passing null
                // aborts inside the core.
                _ = config;
                throw new CoreException(
                    Strings.Get("ErrorCoreStart"),
                    new NotImplementedException(
                        "wooosh-core C# bindings are not generated yet: Config, KeyStore and " +
                        "CoreEventListener have no codec. See Windows/README.md."));
            },
            cancellationToken);

    public Task StopAsync() => Task.Run(() =>
    {
        lock (_gate)
        {
            if (_handle == IntPtr.Zero)
            {
                return;
            }

            UniffiCall.Rust((ref RustCallStatus status) =>
                NativeMethods.uniffi_wooosh_core_fn_method_woooshcore_stop(_handle, ref status));
        }
    });

    public void SetVisibility(CoreVisibility mode) =>
        // TODO(bindings): `mode` is a Visibility enum, lowered as a one-based i32 variant
        // index. The index depends on the declaration order in the Rust enum, so it comes
        // from the generator rather than from a guess here.
        throw NotWired();

    public string? FingerprintPhraseFor(byte[] publicKey) =>
        // Works today: bytes in, string out.
        UniffiSerialization.LiftString(
            UniffiCall.Rust((ref RustCallStatus status) =>
                NativeMethods.uniffi_wooosh_core_fn_func_fingerprint_phrase_for(
                    UniffiSerialization.LowerBytes(publicKey), ref status)));

    public string? DeviceIdFor(byte[] publicKey) =>
        UniffiSerialization.LiftString(
            UniffiCall.Rust((ref RustCallStatus status) =>
                NativeMethods.uniffi_wooosh_core_fn_func_device_id_for(
                    UniffiSerialization.LowerBytes(publicKey), ref status)));

    public Task<IReadOnlyList<TrustedPeerInfo>> TrustedPeersAsync() =>
        // TODO(bindings): returns Vec<TrustedPeer>. Needs the TrustedPeer record codec.
        Task.FromException<IReadOnlyList<TrustedPeerInfo>>(NotWired());

    public Task<bool> RevokePeerAsync(byte[] publicKey) => Task.Run(() =>
    {
        RequireHandle();
        var result = UniffiCall.Rust((ref RustCallStatus status) =>
            NativeMethods.uniffi_wooosh_core_fn_method_woooshcore_revoke_peer(
                _handle, UniffiSerialization.LowerBytes(publicKey), ref status));
        return result != 0;
    });

    public string BeginPairingQr()
    {
        RequireHandle();
        return UniffiSerialization.LiftString(
            UniffiCall.Rust((ref RustCallStatus status) =>
                NativeMethods.uniffi_wooosh_core_fn_method_woooshcore_begin_pairing_qr(_handle, ref status)));
    }

    public PairingCodeInfo? ParsePairingCode(string payload) =>
        // TODO(bindings): parse_pairing_qr returns an Option<PairingQr> record.
        throw NotWired();

    public Task PairWithQrAsync(string payload) => Task.Run(() =>
    {
        RequireHandle();
        // Blocking and slow by design: hints are raced but the reply timeout is 20 s.
        UniffiSerialization.LiftString(
            UniffiCall.Rust((ref RustCallStatus status) =>
                NativeMethods.uniffi_wooosh_core_fn_method_woooshcore_pair_with_qr(
                    _handle, UniffiSerialization.LowerString(payload), ref status)));
    });

    public void RequestSasPairing(string peerId)
    {
        RequireHandle();
        UniffiCall.Rust((ref RustCallStatus status) =>
            NativeMethods.uniffi_wooosh_core_fn_method_woooshcore_request_sas_pairing(
                _handle, UniffiSerialization.LowerString(peerId), ref status));
    }

    public void ConfirmSas(string peerId, bool accepted)
    {
        RequireHandle();
        UniffiCall.Rust((ref RustCallStatus status) =>
            NativeMethods.uniffi_wooosh_core_fn_method_woooshcore_confirm_sas(
                _handle, UniffiSerialization.LowerString(peerId), accepted ? (sbyte)1 : (sbyte)0, ref status));
    }

    public Task<string> ConnectPeerAsync(string addr, byte[]? expectedPublicKey = null) => Task.Run(() =>
    {
        RequireHandle();
        return UniffiSerialization.LiftString(
            UniffiCall.Rust((ref RustCallStatus status) =>
                NativeMethods.uniffi_wooosh_core_fn_method_woooshcore_connect_peer(
                    _handle,
                    UniffiSerialization.LowerString(addr),
                    UniffiSerialization.LowerOptionalBytes(expectedPublicKey),
                    ref status)));
    });

    public Task<TransferId> SendAsync(string peerId, IReadOnlyList<string> filePaths) =>
        // TODO(bindings): `files` is a Vec<String>. The sequence-of-string writer is a
        // three-line addition to UniffiSerialization, but it is only worth adding together
        // with the generated codecs so the whole surface is verified at once.
        Task.FromException<TransferId>(NotWired());

    public void RespondToOffer(TransferId transferId, IReadOnlyList<FileId> acceptedFileIds)
    {
        RequireHandle();
        var ids = acceptedFileIds.Select(id => id.Value).ToList();
        UniffiCall.Rust((ref RustCallStatus status) =>
            NativeMethods.uniffi_wooosh_core_fn_method_woooshcore_respond_to_offer(
                _handle,
                UniffiSerialization.LowerString(transferId.Value),
                UniffiSerialization.LowerUInt32Sequence(ids),
                ref status));
    }

    public void Cancel(TransferId transferId, FileId? fileId = null)
    {
        RequireHandle();
        var lowered = new UniffiSerialization.BufferWriter();
        if (fileId is null)
        {
            lowered.WriteByte(0);
        }
        else
        {
            lowered.WriteByte(1);
            lowered.WriteUInt32(fileId.Value.Value);
        }

        UniffiCall.Rust((ref RustCallStatus status) =>
            NativeMethods.uniffi_wooosh_core_fn_method_woooshcore_cancel(
                _handle,
                UniffiSerialization.LowerString(transferId.Value),
                RustBuffer.FromBytes(lowered.ToArray()),
                ref status));
    }

    /// <summary>Raises an event onto the shell. Called from the core's event thread.</summary>
    internal void Publish(CoreEvent coreEvent) => EventReceived?.Invoke(coreEvent);

    private void RequireHandle()
    {
        if (_handle == IntPtr.Zero)
        {
            throw new CoreException(Strings.Get("ErrorNotStarted"));
        }
    }

    private static CoreException NotWired() =>
        new(Strings.Get("ErrorCoreStart"),
            new NotImplementedException(
                "This call needs the generated wooosh-core C# codec. See Windows/README.md."));

    public void Dispose()
    {
        lock (_gate)
        {
            if (_handle == IntPtr.Zero)
            {
                return;
            }

            UniffiCall.Rust((ref RustCallStatus status) =>
                NativeMethods.uniffi_wooosh_core_fn_free_woooshcore(_handle, ref status));
            _handle = IntPtr.Zero;
        }
    }
}

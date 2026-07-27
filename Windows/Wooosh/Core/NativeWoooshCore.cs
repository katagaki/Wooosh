using Wooosh.Core.Interop;
using Wooosh.Localization;

namespace Wooosh.Core;

/// <summary>The real core, over <c>wooosh_core.dll</c>. Members touching the core's own
/// records, enums or callback interfaces need generated codecs whose byte layout and VTable
/// order must match Rust exactly, so they throw rather than invent a result
/// (Windows/README.md); the rest work today through <see cref="UniffiSerialization"/>.</summary>
public sealed class NativeWoooshCore : IWoooshCore
{
    private readonly object _gate = new();
    private IntPtr _handle = IntPtr.Zero;

    public event Action<CoreEvent>? EventReceived;

    public string? DeviceId { get; private set; }

    public string? FingerprintPhrase { get; private set; }

    public string? ListenAddr { get; private set; }

    /// <summary>Checked first so a missing or contract-mismatched DLL fails clearly at
    /// startup rather than crashing later.</summary>
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
        // Off the UI thread, always: start() boots inline and calls KeyStore.load_identity
        // synchronously, which on Windows means DPAPI and possibly Windows Hello (DESIGN.md §4).
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

                // TODO(bindings): needs a lowered Config plus KeyStore and CoreEventListener
                // callback objects; passing null for those two arguments aborts in the core.
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
        // TODO(bindings): `mode` lowers to a variant index that depends on the Rust enum's
        // declaration order, so it must come from the generator.
        throw NotWired();

    public string? FingerprintPhraseFor(byte[] publicKey) =>
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
        // Blocking by design: hints are raced but the reply timeout is 20 s.
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
        // TODO(bindings): `files` is a Vec<String>, added with the generated codecs.
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

    // The internet path (PROTOCOL.md §9) is exported as UniFFI async functions, polled
    // through a Rust future handle, so it needs the async scaffolding the codecs bring.

    public Task<string> BeginInternetTicketAsync() =>
        Task.FromException<string>(NotWired());

    public void EndInternetTicket() => throw NotWired();

    public Task<string> RedeemTicketAsync(string ticket) =>
        Task.FromException<string>(NotWired());

    /// <summary>Called from the core's event thread.</summary>
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

# Wooosh for Windows

WinUI 3 shell for [Wooosh](../README.md). Same shared Rust core as the Apple and Android
shells, same protocol, native UI.

> **This has never been compiled or run.** It was written on a Mac, which has no Visual
> Studio, no Windows SDK and no .NET Windows targets, so nothing here has been through a
> compiler, a XAML compiler, MakePri, or MSIX packaging. Treat it as a starting point that
> is meant to be correct by inspection, not as a working build. The
> [What still needs doing](#what-still-needs-doing) section is honest about the gaps, and
> [Unverified assumptions](#unverified-assumptions) lists the specific things a Windows
> machine has to confirm first.

## Prerequisites

| Thing | Version |
|---|---|
| Visual Studio 2022 | 17.10 or newer, with the **.NET Desktop Development** workload and its **Windows App SDK C# Templates** component |
| Windows SDK | 10.0.26100 (the `TargetFramework` moniker); minimum supported OS is 10.0.17763 |
| .NET | 8.0 (LTS) |
| Windows App SDK | 1.7 (`Microsoft.WindowsAppSDK` 1.7.250606001), pulled by NuGet |
| Rust | only if you are building the core: see [`Core/`](../Core) |

Also enable **Developer Mode** in Windows Settings. A packaged app cannot be deployed
locally without it.

## Open and build

```
Windows\Wooosh.sln
```

Pick `x64` (or `ARM64` on an Arm machine) and press F5. There is no `AnyCPU`
configuration: the Windows App SDK ships per-architecture native bits, so WinUI 3 projects
target an explicit architecture.

The first build restores three packages: `Microsoft.WindowsAppSDK`,
`Microsoft.Windows.SDK.BuildTools` and `System.Security.Cryptography.ProtectedData`.

The app will launch, discover other Wooosh devices on the LAN, and show a red banner saying
it could not start. That is expected and correct: the transfer engine is not linked yet.

## Layout

```
Windows/
├── Wooosh.sln
├── README.md
└── Wooosh/
    ├── Wooosh.csproj
    ├── app.manifest                    PerMonitorV2 DPI
    ├── Package.appxmanifest            identity, capabilities, Share Target
    ├── App.xaml / App.xaml.cs          app lifetime, share-target activation
    ├── MainWindow.xaml / .cs           title bar, frame, minimize-to-tray
    ├── Assets/                         placeholder MSIX logos (see below)
    ├── Core/
    │   ├── IWoooshCore.cs              the DESIGN.md §4 contract
    │   ├── CoreModels.cs               Config, PeerRef, TrustedPeerInfo, FileMeta, …
    │   ├── CoreEvent.cs                the event stream
    │   ├── NativeWoooshCore.cs         the only implementation, over wooosh_core.dll
    │   └── Interop/
    │       ├── NativeMethods.cs        P/Invoke declarations for the UniFFI ABI
    │       ├── RustBuffer.cs           RustBuffer / RustCallStatus / call wrapper
    │       └── UniffiSerialization.cs  the UniFFI buffer format
    ├── Discovery/
    │   ├── DiscoveryController.cs      settings -> advertiser, browser -> registry
    │   ├── DnssdAdvertiser.cs          mDNS advertise
    │   ├── DnssdBrowser.cs             mDNS browse + 2 s scan
    │   └── TxtRecord.cs                the PROTOCOL.md §3.1 TXT record
    ├── Peers/
    │   ├── DeviceType.cs               the `dt` enum and its glyphs
    │   ├── Peer.cs                     one row, observable, mutated in place
    │   └── PeerRegistry.cs             the ordering and staleness rules
    ├── Platform/
    │   ├── StorageRouter.cs            Downloads, no-overwrite naming, Mark of the Web
    │   ├── PowerManagement.cs          SetThreadExecutionState during transfers
    │   ├── TrayIcon.cs                 Shell_NotifyIcon, open / quit
    │   └── DpapiKeyStore.cs            DPAPI-wrapped identity key
    ├── Settings/SettingsRepository.cs
    ├── Localization/
    │   ├── Strings.cs                  the only way text enters the UI
    │   ├── PluralRules.cs              CLDR categories for the 14 shipped locales
    │   ├── Formatters.cs               byte sizes and durations
    │   ├── LocalizeExtension.cs        {loc:Localize Key=…}
    │   └── Converters.cs
    ├── ViewModels/
    │   ├── MainViewModel.cs
    │   ├── TransferViewModel.cs
    │   └── ObservableObject.cs
    ├── Views/
    │   ├── DeviceListPage.xaml / .cs
    │   ├── SettingsPage.xaml / .cs
    │   ├── PairingPage.xaml / .cs
    │   ├── IncomingOfferDialog.xaml / .cs
    │   └── TransferProgressControl.xaml / .cs
    └── Strings/
        ├── en-US/Resources.resw        + ja, ko, zh-Hans, zh-Hant, de, es, fr,
        └── …                             it, nl, pt-BR, pl, ru, sv
```

## Decisions worth knowing about

### mDNS: the WinRT DNS-SD API, not a hand-rolled responder

Discovery is native per platform (DESIGN.md §2) specifically so no shell ships a second
mDNS responder to fight the system one. Windows has had an mDNS responder in `dnsapi.dll`
since Windows 10 1703 and `Windows.Networking.ServiceDiscovery.Dnssd` plus a
`DeviceInformation` watcher is its public surface, so it is the direct counterpart of
`NsdManager` on Android and `NWListener`/`NWBrowser` on Apple. Rolling one on
`System.Net.Sockets` would mean binding UDP 5353 next to the OS responder and
re-implementing DNS record encoding: a few hundred lines of unverifiable parsing for
something the OS already publishes correctly. A third-party mDNS NuGet package has the same
problem plus a dependency this project would then own.

The awkward part, documented in `DnssdAdvertiser.cs`: `DnssdServiceInstance` can only be
registered against a `StreamSocketListener` or a `DatagramSocket`, and Wooosh's listener is
a QUIC UDP socket owned by the core. TCP and UDP port numbers are separate namespaces, so
the advertiser binds a `StreamSocketListener` on the same port *number* purely to have
something to attach the registration to. It accepts nothing. DNS-SD convention already puts
the QUIC UDP port in a `_tcp` SRV record (PROTOCOL.md §1), so this is consistent with the
other shells on the wire.

Staleness needs a heartbeat, and watcher `Updated` events fire on change rather than on
every announce, so `DnssdBrowser` also runs a 2 s `FindAllAsync` scan. That is what
PROTOCOL.md §3.3 means by "announce/scan expected every ≤ 2 s", and it is what makes the
10 s silence threshold meaningful.

### FFI: hand-written P/Invoke against the UniFFI ABI

UniFFI ships first-party generators for Kotlin, Swift and Python only. The C# generator
(`uniffi-bindgen-cs`, NordSecurity) is the right long-term answer but tracks UniFFI
versions with a lag, and the core is on UniFFI 0.29 today. Rather than block the whole
shell on that, `Core/Interop/NativeMethods.cs` declares the scaffolding ABI by hand. The
symbol names and signatures were read straight out of the generated
`Core/bindings/swift/wooosh_coreFFI.h` in this repository, so they are the real contract
rather than a guess, and the UniFFI buffer format in `UniffiSerialization.cs` is complete
for primitives.

**What still has to be wired up**, and why it was not guessed at:

1. **Record and enum codecs.** `Config`, `PeerRef`, `TrustedPeer`, `FileMeta`, `CoreEvent`,
   `Visibility` and `CoreError` each serialise as their fields in declaration order.
   Transcribing that order from memory produces code that compiles, runs, and reads a
   peer's public key from the wrong offset. This has to come from a generator or from
   reading `Core/bindings/kotlin/uniffi/wooosh_core/wooosh_core.kt` line by line.
2. **Callback VTables.** `KeyStore` and `CoreEventListener` are foreign-implemented traits.
   Rust calls back through a table of function pointers registered by
   `uniffi_wooosh_core_fn_init_callback_vtable_*`, plus a handle map keeping the managed
   objects alive. Field order in that table is generated, not documented, and getting it
   wrong crashes inside the core.

Two ways forward, in order of preference:

- Run `uniffi-bindgen-cs` against `wooosh-core` and drop the generated file into `Core/`.
  Add a step to `Core/build-bindings.sh` next to the Kotlin and Swift ones. If the
  generator does not support 0.29 yet, this is the thing to fix.
- Or fill in the codecs by hand from the Kotlin bindings, which are the same buffer format
  and are already in the tree.

Either way, `IWoooshCore` is the seam and nothing above it has to change.

**There is deliberately no mock implementation.** The Apple and Android shells both had one
and both had it removed. A shell that can fabricate peers and transfers makes every
screenshot, demo and bug report ambiguous. When the core is missing, Wooosh says so.

### Localization

All 14 locales are populated. The `.resw` files were generated from the shipped Android
translations (`Android/app/src/main/res/values*/strings.xml`), so the two shells use one
term per concept, and Android positional specifiers (`%1$s`) were rewritten to .NET
composite format (`{0}`). **Nothing fell back to English**: every string that has an
Android counterpart carries its real translation in all 14 locales.

Six strings have no Android counterpart because they are Windows-only, and were translated
for this shell rather than reused: `TrayOpen`, `TrayQuit`, `SettingsSectionBackground`,
`SettingsKeepRunning`, `SettingsKeepRunningDesc` and `ShareTargetTitle`. They are short and
mechanical, but they have not been reviewed by anyone else, so they are the first thing to
check if a translator ever looks at this.

Two mechanisms are worth flagging:

- **No `x:Uid`.** `x:Uid` forces resource keys to be named after the XAML element and the
  property it sets (`EmptyTitleText.Text`), which makes it impossible to keep key names
  aligned with the Android and Apple catalogues. `{loc:Localize Key=…}`, a nine-line markup
  extension, keeps one key name per concept across all three shells.
- **Plurals are selected in code.** Windows resource files have no equivalent of Android's
  `<plurals>` or an Apple String Catalog's plural variations. `PluralRules.cs` implements
  the CLDR cardinal categories for the shipped locales and the `.resw` carries one entry per
  form (`OfferTitle.One`, `.Few`, `.Many`, `.Other`). Polish and Russian have four forms;
  an English-shaped `count == 1` test is wrong in both.

### Accent colour

There is no brand colour anywhere. Everything tinted uses `AccentFillColorDefaultBrush`,
`AccentTextFillColorPrimaryBrush` and `AccentButtonStyle`, so Wooosh follows the user's
Windows accent (DESIGN.md §5). `App.xaml` aliases them so a view never reaches for a
literal.

## What is actually implemented

- **The device list rules.** `PeerRegistry` is complete: first-discovery ordering, an
  append-only `ObservableCollection` that is never re-sorted, a 10 s stale threshold, and
  rows that grey out and stop responding *in place*. `DeviceListPage.xaml` binds it with no
  sort descriptor, no grouping and no `CollectionViewSource`.
- **The TXT record.** `TxtRecord` builds and parses exactly `v`, `rid`, `dn`, `dt`, `p`,
  `vis` per PROTOCOL.md §3.1, advertises `dt = windows`, omits `dt` rather than guessing,
  and treats unknown device types as unknown.
- **Discovery.** Advertiser, browser, 2 s scan, self-filtering by `rid`, debounced
  re-registration on settings changes.
- **Storage routing.** Downloads, ` (2)` / ` (3)` collision naming that never overwrites,
  and the `Zone.Identifier` Mark-of-the-Web stream on every received file.
- **Keep-awake.** `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)`, refcounted
  across concurrent transfers, released on done or error. Display sleep is deliberately not
  blocked.
- **Minimize to tray.** `Shell_NotifyIcon` with an Open / Quit menu, window close hides
  instead of exiting, and a setting to turn that off.
- **DPAPI key storage.** Complete, and testable on its own. Not yet reachable by the core.
- **Localization.** 14 locales, plural rules, formatters, and no literal display string
  anywhere in the views.
- **The FFI seam.** `IWoooshCore` mirrors DESIGN.md §4 in full. The ABI declarations and
  primitive codec are real; the calls that only need strings, bytes and booleans
  (`fingerprint_phrase_for`, `device_id_for`, `begin_pairing_qr`, `connect_peer`,
  `revoke_peer`, `confirm_sas`, `respond_to_offer`, `cancel`) are written and should work as
  soon as the DLL is present.

## What still needs doing

Everything below is marked with a `TODO(...)` comment at the place it belongs.

1. **`TODO(bindings)`: the record codecs and callback VTables.** Nothing transfers until
   this lands. Blocks `StartAsync`, `TrustedPeersAsync`, `SendAsync`, `ParsePairingCode`,
   `SetVisibility`, the whole event stream, and typed core errors (`RustCallStatus.Error`
   currently loses the variant, so `PAIRING_REQUIRED` and "the network dropped" are
   indistinguishable, which PROTOCOL.md §4.1.2 calls a conformance bug).
2. **`wooosh_core.dll` itself.** Build it per architecture
   (`cargo build --release --target x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`)
   and restore the commented-out item group in `Wooosh.csproj` so it travels with the
   package. Do not commit the DLL.
3. **`TODO(send)`: the send flow.** File picker, then `connect_peer` with the pinned key
   from `trusted_peers()`, then `send`. Both calls block and must run off the UI thread.
4. **`TODO(share)`: Share Target activation.** The manifest declaration is in place; the
   handler in `App.xaml.cs` only logs. It has to stage the `StorageItems` into the local
   folder and report the share operation complete *before* the transfer finishes, or
   Windows keeps the source app blocked for the whole send.
5. **File Explorer context-menu verb.** DESIGN.md §8 asks for "Send with Wooosh" on
   right-click. On Windows 11 that is a sparse-package `IExplorerCommand` COM server, which
   is a project of its own. `Package.appxmanifest` documents what it needs. An XML-only stub
   would register a verb that does nothing, which is worse than no verb.
6. **`TODO(pairing)`: QR rendering.** There is no QR encoder in the box on Windows. The
   pairing page shows the payload as copyable text and leaves a placeholder where the code
   goes. Filling it means picking a dependency, which is a real decision rather than a stub.
7. **`TODO(views)`: the remaining dialogs.** `IncomingOfferDialog` is written but never
   shown; SAS comparison, pairing result and the `KeyChanged` warning have no dialog yet.
   `KeyChanged` in particular must be prominent and must never offer a silent re-pin.
8. **`TODO(DESIGN.md §6)`: the `Wooosh/<date>` subfolder** for receives of more than 20
   files.
9. **Placeholder art.** `Assets/*.png` are flat grey rectangles at the right dimensions so
   the package builds. Replace them, and add the scaled variants
   (`.scale-100/125/150/200/400`) Windows expects.
10. **Package identity.** `Publisher="CN=Tsubuzaki"` is a placeholder for a local
    self-signed test certificate. Replace it before doing anything but F5 on a dev machine.
11. **UDP discovery fallback** (PROTOCOL.md §3.2, broadcast on 44777). Not implemented here.
    Neither is it implemented on Android, so the shells are consistent.
12. **No tests.** There is no test project. `PeerRegistry`, `TxtRecord`, `PluralRules` and
    `StorageRouter`'s collision naming are all pure enough to unit test and are the obvious
    first candidates.

## Unverified assumptions

Things a Windows machine should confirm early, because being wrong about them is a build
failure rather than a bug:

- **Package versions.** `Microsoft.WindowsAppSDK` 1.7.250606001 and
  `Microsoft.Windows.SDK.BuildTools` 10.0.26100.4188 were chosen as known-good, not
  verified against NuGet. If either fails to restore, take the nearest available and adjust
  `TargetFramework` to match.
- **`.resw` discovery.** The project relies on the default `PRIResource` glob that the
  Windows App SDK targets enable, so `Strings/<locale>/Resources.resw` is not listed in the
  `.csproj`. If resources do not resolve at runtime, set
  `<EnableDefaultPRIResourceItems>false</EnableDefaultPRIResourceItems>` and list them
  explicitly. Adding `<PRIResource>` items *without* disabling the default first produces
  duplicate-item errors.
- **`ms-resource:///Resources/ShareTargetTitle`** in `Package.appxmanifest`. The resource map
  path is the part most likely to be wrong; a bad `ms-resource` reference fails packaging,
  not runtime.
- **`ResourceLoader` default subtree.** `Strings.cs` uses `new ResourceLoader()`, which
  resolves against the `Resources` subtree. Correct for `Resources.resw`, unverified.
- **The `StreamSocketListener` port trick** in `DnssdAdvertiser`. Binding TCP on the same
  number as the core's UDP socket should be fine, and the SRV port should come out right,
  but this is the single most likely thing to behave differently than described.
- **DNS-SD property names.** `System.Devices.Dnssd.TextAttributes` is read as `string[]` of
  `key=value`, and `System.Devices.IpAddress` as `string[]`. If either arrives in another
  shape, `DnssdBrowser.Ingest` silently drops every peer.
- **`DnssdServiceInstance.DnssdServiceInstanceName`** is assumed to be the post-registration
  instance name. Only used as a secondary self-filter; the `rid` check is what actually
  matters.
- **Window subclassing.** `TrayIcon` installs a `WndProc` over the WinUI window's HWND via
  `SetWindowLongPtr`. This works, but WinUI also subclasses that window, so the ordering
  relative to `ExtendsContentIntoTitleBar` should be sanity-checked.
- **`AppWindow.Closing` cancellation.** Used to hide instead of exit. Confirm that
  cancelling it leaves the process healthy rather than in a half-closed state.
- **x86.** `SetWindowLongPtrW` does not exist in 32-bit `user32`, so `TrayIcon` branches to
  `SetWindowLongW`. The x86 configuration has had no other thought put into it; x64 and
  ARM64 are the intended targets.

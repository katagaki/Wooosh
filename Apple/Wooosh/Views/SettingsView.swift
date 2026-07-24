import SwiftUI

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    @State private var showingPairSheet = false

    var body: some View {
        #if os(macOS)
        // Presented as the ⌘, Settings scene: the window supplies the chrome,
        // so no NavigationStack, no title bar of our own, no Done button.
        form
            .formStyle(.grouped)
            .frame(width: 520)
            .frame(minHeight: 480)
            .sheet(isPresented: $showingPairSheet) {
                PairDeviceView()
            }
        #else
        NavigationStack {
            form
                .formStyle(.grouped)
                .navigationTitle(L.t("action_settings"))
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        DoneButton { dismiss() }
                    }
                }
                .sheet(isPresented: $showingPairSheet) {
                    PairDeviceView()
                }
        }
        #endif
    }

    private var form: some View {
        @Bindable var model = model
        return Form {
            Section(L.t("settings_section_device")) {
                TextField(L.t("settings_display_name"), text: $model.displayName)
            }
            Section {
                Picker(L.t("settings_section_who_can_see"), selection: $model.visibility) {
                    ForEach(Visibility.allCases) { mode in
                        Text(mode.label).tag(mode)
                    }
                }
            } header: {
                Text(L.t("settings_section_who_can_see"))
            } footer: {
                Text(visibilityFooter)
            }
            Section {
                Button(L.t("action_pair_device"), systemImage: "qrcode") {
                    showingPairSheet = true
                }
            } header: {
                Text(L.t("settings_section_pairing"))
            } footer: {
                Text(L.t("settings_pairing_footer"))
            }
            pairedDevicesSection
            Section {
                LabeledContent(L.t("settings_device_id")) {
                    Text(model.deviceIDString)
                        .font(.system(.caption, design: .monospaced))
                        .textSelection(.enabled)
                }
                if !model.fingerprintPhrase.isEmpty {
                    LabeledContent(L.t("verify_phrase_title")) {
                        Text(model.fingerprintPhrase)
                            .font(.system(.caption, design: .monospaced))
                            .textSelection(.enabled)
                    }
                }
            } header: {
                Text(L.t("settings_section_this_device"))
            } footer: {
                Text(L.t("verify_phrase_body"))
            }
            if let error = model.startupError {
                Section(L.t("settings_section_problem")) {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                }
            }
            #if DEBUG
            debugSection
            #endif
        }
    }

    /// The footer describes whichever option is selected, so the consequence
    /// of the choice is on screen instead of only in the option list.
    private var visibilityFooter: String {
        switch model.visibility {
        case .everyone: L.t("settings_visibility_everyone_desc")
        case .pairedOnly: L.t("settings_visibility_paired_desc")
        case .off: L.t("settings_visibility_off_desc")
        }
    }

    #if DEBUG
    /// Engine switch: the mock keeps the scripted UI demo available without a
    /// peer on the network. Ships DEBUG-only; release always uses the core.
    @ViewBuilder
    private var debugSection: some View {
        @Bindable var model = model
        Section {
            Picker("Engine", selection: Binding(
                get: { model.backend },
                set: { model.switchBackend(to: $0) }
            )) {
                ForEach(CoreBackend.allCases) { backend in
                    Text(backend.label).tag(backend)
                }
            }
        } header: {
            Text("Debug")
        } footer: {
            Text("The mock engine replays scripted transfers so the UI can be demoed offline.")
        }
    }
    #endif

    @ViewBuilder
    private var pairedDevicesSection: some View {
        if !model.trustStore.devices.isEmpty {
            Section {
                // The list is the core's `trustedPeers()`, keyed by DeviceID.
                ForEach(model.trustStore.devices) { device in
                    PairedDeviceRow(
                        device: device,
                        keyChanged: model.trustStore.hasKeyChanged(deviceID: device.deviceID)
                    ) {
                        model.revoke(device: device)
                    }
                }
            } header: {
                Text(L.t("settings_section_paired_devices"))
            } footer: {
                Text(L.t("settings_paired_devices_footer"))
            }
        }
    }
}

private struct PairedDeviceRow: View {
    let device: TrustedPeerInfo
    let keyChanged: Bool
    let revoke: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: device.symbolName)
                .foregroundStyle(Color.accentColor)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 2) {
                Text(device.displayName)
                if keyChanged {
                    Label(L.t("keychanged_row_warning"), systemImage: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(.red)
                } else {
                    // The phrase itself is a shared verification artifact and
                    // is never translated; only the label around it is.
                    Text(device.fingerprint)
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(.secondary)
                    Text(L.f("settings_paired_on", device.pairedAt.formatted(date: .abbreviated, time: .omitted)))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            Button(L.t("action_unpair"), role: .destructive, action: revoke)
                .buttonStyle(.borderless)
                .foregroundStyle(.red)
        }
        .contextMenu {
            Button(L.t("action_unpair"), role: .destructive, action: revoke)
        }
    }
}

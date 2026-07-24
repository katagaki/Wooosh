import SwiftUI
#if os(macOS)
import AppKit
#endif

struct DeviceListView: View {
    @Environment(AppModel.self) private var model

    @State private var showingSettings = false
    @State private var showingPairSheet = false
    @State private var selectedPeer: Peer?
    /// Local copy of the pending offer so the sheet survives acceptance
    /// (accepting clears `transfers.pendingOffer` but keeps this presented
    /// as the live progress view).
    @State private var incomingOffer: Transfer?
    @State private var showingKeyChangedAlert = false

    var body: some View {
        @Bindable var model = model
        NavigationStack {
            deviceList
                .navigationTitle(L.t("Wooosh"))
                .toolbar {
                    #if DEBUG
                    ToolbarItem {
                        debugMenu
                    }
                    #endif
                    ToolbarItem {
                        Button(L.t("action_refresh"), systemImage: "arrow.clockwise") {
                            model.refresh()
                        }
                    }
                    // Splits the bar into separate Liquid Glass groups rather
                    // than one long capsule of unrelated controls.
                    ToolbarSpacer(.fixed)
                    #if os(macOS)
                    // On the Mac, pairing is a window action; Settings is not
                    // in the window at all — it is the ⌘, Settings scene.
                    ToolbarItem {
                        Button(L.t("action_pair_device"), systemImage: "qrcode") {
                            showingPairSheet = true
                        }
                    }
                    #else
                    ToolbarItem {
                        Button(L.t("action_settings"), systemImage: "gearshape") {
                            showingSettings = true
                        }
                    }
                    #endif
                }
                // The floating layer: glass belongs here, above the content,
                // not painted behind the rows.
                .safeAreaInset(edge: .bottom) {
                    floatingBar
                }
                // Pairing started from a row's context menu has no sheet of
                // its own, and the connect can take many seconds. Without
                // this the app looks hung and users force-quit it.
                .overlay {
                    if case .connecting(let peerName) = model.pairingPhase, !showingPairSheet {
                        PairingProgressOverlay(peerName: peerName) {
                            model.cancelPairing()
                        }
                    }
                }
                .alert(L.t("pairing_failed_title"), isPresented: pairingFailedBinding) {
                    Button(L.t("action_ok"), role: .cancel) { model.resetPairingPhase() }
                } message: {
                    Text(model.pairingPhase.failureMessage ?? L.t("error_pairing_failed"))
                }
            #if os(iOS)
            .sheet(isPresented: $showingSettings) {
                SettingsView()
            }
            #endif
            .sheet(isPresented: $showingPairSheet) {
                PairDeviceView()
            }
            .sheet(item: $selectedPeer) { peer in
                SendTransferSheet(peer: peer)
            }
            .sheet(item: $incomingOffer, onDismiss: {
                // Present the next queued offer, if one arrived while the
                // previous sheet was showing progress.
                if let next = model.transfers.pendingOffer {
                    incomingOffer = next
                }
            }) { offer in
                IncomingOfferSheet(offer: offer)
            }
            .sheet(item: $model.activeSAS) { request in
                SASSheet(request: request)
            }
            .alert(
                L.t("keychanged_title"),
                isPresented: $showingKeyChangedAlert,
                presenting: model.keyChangeWarning
            ) { _ in
                Button(L.t("action_pair_again"), role: .destructive) {
                    model.keyChangeWarning = nil
                    showingPairSheet = true
                }
                Button(L.t("action_not_now"), role: .cancel) {
                    model.keyChangeWarning = nil
                }
            } message: { warning in
                // Both fingerprints come from the core, so the user can read
                // them aloud against the other device instead of guessing.
                Text(keyChangeMessage(warning))
            }
        }
        .onChange(of: model.transfers.pendingOffer) { _, offer in
            if let offer, incomingOffer == nil {
                incomingOffer = offer
            }
        }
        .onChange(of: model.keyChangeWarning) { _, warning in
            showingKeyChangedAlert = warning != nil
        }
        .onOpenURL { url in
            model.handleIncomingURL(url)
        }
        #if DEBUG
        // Sheet presentation lives in this view's own state, so the demo
        // harness reaches it through here rather than from the model.
        .onChange(of: model.debugScreen) { _, screen in
            switch screen {
            case .pair, .pairingConnecting, .pairingFailedInSheet:
                showingPairSheet = true
            case .send:
                selectedPeer = model.registry.peers.first { !$0.isStale }
            case .settings:
                #if os(macOS)
                // Proves the ⌘, Settings *scene* opens — there is no in-window
                // settings screen on the Mac any more.
                NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
                #else
                showingSettings = true
                #endif
            case .devices, .offer, .sas, .pairingFailed, .pairingOverlay, nil:
                break
            }
        }
        #endif
        #if os(macOS)
        .frame(minWidth: 380, minHeight: 460)
        #endif
    }

    /// A failure only becomes an alert here when no pairing sheet is up to
    /// show it in place.
    private var pairingFailedBinding: Binding<Bool> {
        Binding(
            get: { model.pairingPhase.failureMessage != nil && !showingPairSheet },
            set: { presented in
                if !presented { model.resetPairingPhase() }
            }
        )
    }

    @ViewBuilder
    private var deviceList: some View {
        if model.registry.peers.isEmpty {
            emptyState
        } else {
            peerList
        }
    }

    /// The alert body, assembled from independently translated blocks: one
    /// explanation, two labelled verification phrases, one instruction. Each
    /// block is a complete unit of its own, so nothing here builds a sentence
    /// out of fragments; the blank lines are layout, not grammar. The phrases
    /// themselves are never translated (they must read identically on both
    /// devices, or the comparison the alert asks for is meaningless).
    private func keyChangeMessage(_ warning: KeyChangeWarning) -> String {
        var blocks = [L.f("keychanged_body", warning.peer.displayName)]
        if !warning.expectedFingerprint.isEmpty {
            blocks.append(L.t("keychanged_expected_label") + "\n" + warning.expectedFingerprint)
            let presented = warning.presentedFingerprint ?? ""
            blocks.append(presented.isEmpty
                          ? L.t("keychanged_presented_unknown")
                          : L.t("keychanged_presented_label") + "\n" + presented)
        }
        blocks.append(L.t("keychanged_advice"))
        return blocks.joined(separator: "\n\n")
    }

    private var peerList: some View {
        // Rows are rendered verbatim in registry order: appended at first
        // sighting, never re-sorted, never removed (DESIGN.md §5). No
        // `.animation` on the collection, so nothing can slide a row out from
        // under a finger.
        List(model.registry.peers) { peer in
            PeerRowView(
                peer: peer,
                isPaired: model.isPaired(peer)
            ) {
                selectedPeer = peer
            }
            .contextMenu {
                Button(L.t("action_send_files"), systemImage: "paperplane") {
                    selectedPeer = peer
                }
                .disabled(peer.isStale)
                if !model.isPaired(peer) {
                    Button(L.t("action_pair_this_device"), systemImage: "link") {
                        model.requestSASPairing(with: peer)
                    }
                    .disabled(peer.isStale)
                }
                Button(L.t("action_show_my_code"), systemImage: "qrcode") {
                    showingPairSheet = true
                }
            }
        }
        #if os(iOS)
        // The one sanctioned way rows leave the list (DESIGN.md §5).
        .refreshable {
            model.refresh()
        }
        #endif
    }

    /// Floating controls above the list. Either the armed share batch, or —
    /// on iOS, where there is no menu bar to hang it off — the standing
    /// "pair a device" affordance.
    @ViewBuilder
    private var floatingBar: some View {
        if let batch = model.pendingShareBatch {
            shareBatchBar(batch)
        } else {
            #if os(iOS)
            if !model.registry.peers.isEmpty {
                Button(L.t("action_pair_device"), systemImage: "qrcode") {
                    showingPairSheet = true
                }
                .buttonStyle(.glass)
                .controlSize(.large)
                .padding(.bottom, 8)
            }
            #endif
        }
    }

    private func shareBatchBar(_ batch: ShareBatch) -> some View {
        HStack(spacing: 12) {
            Image(systemName: "square.and.arrow.up.circle.fill")
                .font(.title2)
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 2) {
                Text(L.f("share_batch_ready", batch.urls.count))
                    .font(.subheadline.weight(.semibold))
                Text(L.t("share_batch_tap_hint"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            Button(L.t("action_cancel"), systemImage: "xmark") {
                model.discardPendingBatch()
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.glass)
            .controlSize(.regular)
        }
        .padding(.leading, 16)
        .padding(.trailing, 8)
        .padding(.vertical, 10)
        // A real floating pane, so it takes real glass — replacing the old
        // hand-rolled `.quaternary` fill that fought the system chrome.
        .glassEffect(.regular, in: .rect(cornerRadius: 26))
        .padding(.horizontal, 16)
        .padding(.bottom, 8)
    }

    private var emptyState: some View {
        ContentUnavailableView {
            Label(L.t("empty_title"), systemImage: "dot.radiowaves.left.and.right")
        } description: {
            Text(emptyStateDescription)
        } actions: {
            Button(L.t("action_pair_device"), systemImage: "qrcode") {
                showingPairSheet = true
            }
            .buttonStyle(.glassProminent)
            .controlSize(.extraLarge)
        }
    }

    /// Two complete sentences, each translated on its own and joined by a
    /// space: what the list does, then what nearby devices can see right now.
    private var emptyStateDescription: String {
        let visibility: String
        switch model.visibility {
        case .everyone: visibility = L.t("empty_visibility_everyone")
        case .pairedOnly: visibility = L.t("empty_visibility_paired")
        case .off: visibility = L.t("empty_visibility_off")
        }
        return L.t("empty_body") + " " + visibility
    }

    #if DEBUG
    /// Drives MockCore's scripted flows. The simulations only mean anything
    /// on the mock engine, so switching is part of the same menu.
    private var debugMenu: some View {
        Menu("Debug", systemImage: "ladybug") {
            Picker("Engine", selection: Binding(
                get: { model.backend },
                set: { model.switchBackend(to: $0) }
            )) {
                ForEach(CoreBackend.allCases) { backend in
                    Text(backend.label).tag(backend)
                }
            }
            Section("Device List") {
                Button("Add Nearby Devices") {
                    model.debugPopulateNearbyDevices()
                }
            }
            Section("Mock Simulations") {
                Button("Simulate Incoming Offer") {
                    model.debugSimulateIncomingOffer()
                }
                Button("Simulate Pairing Request (SAS)") {
                    model.debugSimulateIncomingSASRequest()
                }
                Button("Simulate Key Change") {
                    model.debugSimulateKeyChanged()
                }
            }
            .disabled(model.backend != .mock)
        }
    }
    #endif
}

struct PeerRowView: View {
    let peer: Peer
    let isPaired: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 14) {
                Image(systemName: peer.symbolName)
                    .font(.title2)
                    .foregroundStyle(.tint)
                    .frame(width: 34)
                    // The glyph is the only place the row states what kind of
                    // device this is, so name it for VoiceOver.
                    .accessibilityLabel(DeviceIcon.label(for: peer.deviceKind))
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 5) {
                        Text(peer.displayName)
                            .font(.body)
                            .lineLimit(1)
                        if isPaired {
                            // Paired checkmark, not a separate section — a
                            // section would move rows (DESIGN.md §5).
                            Image(systemName: "checkmark.seal.fill")
                                .font(.caption)
                                .foregroundStyle(.green)
                                .accessibilityLabel(L.t("peer_badge_paired"))
                        }
                    }
                    Text(stateLine)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 4)
            .frame(minHeight: 44)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        // Stale peers gray out in place — same row, same height, same
        // position, just disabled (DESIGN.md §5).
        .opacity(peer.isStale ? 0.4 : 1)
        .disabled(peer.isStale)
    }

    private var stateLine: String {
        if peer.isStale { return L.t("peer_state_away") }
        return L.t(isPaired ? "peer_state_ready_paired" : "peer_state_ready")
    }
}

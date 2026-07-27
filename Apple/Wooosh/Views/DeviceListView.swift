import SwiftUI

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
    @State private var showingOtherDevice = false

    var body: some View {
        @Bindable var model = model
        NavigationStack {
            deviceList
                .navigationTitle(L.t("Wooosh"))
                .toolbar {
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
            // One row, one sheet: sending and receiving are segments inside it,
            // not a question asked before it opens (PROTOCOL.md §9.4).
            .sheet(isPresented: $showingOtherDevice) {
                OtherDeviceView()
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
        List {
            // Pinned FIRST, never appended: a synthetic row at the end would
            // be pushed down by every new discovery, which is the moving-target
            // mis-tap the list rules exist to prevent. At the top it is a fixed
            // target and the real rows keep their order below it.
            if model.relayPreference.internetEnabled {
                // Its own section: it is not a discovered device and must not
                // read as one sitting among them.
                Section {
                    OtherDeviceRow { showingOtherDevice = true }
                }
            }
            Section {
            ForEach(model.registry.peers) { peer in
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
            // No devices nearby is exactly when someone reaches for the
            // internet path, so it must be offered here too.
            if model.relayPreference.internetEnabled {
                Button(L.t("other_device_title"), systemImage: "globe") {
                    showingOtherDevice = true
                }
                .buttonStyle(.glass)
                .controlSize(.large)
            }
        }
    }

    /// The one row that is not a discovered device: the entry point to the
    /// internet path (PROTOCOL.md §9).
    private struct OtherDeviceRow: View {
        let action: () -> Void

        var body: some View {
            Button(action: action) {
                HStack(spacing: 14) {
                    Image(systemName: "globe")
                        .font(.title2)
                        .foregroundStyle(.tint)
                        .frame(width: 34)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(L.t("other_device_title"))
                            .font(.body)
                            .lineLimit(1)
                        Text(L.t("other_device_subtitle"))
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
        // Said plainly, because the row is otherwise indistinguishable from a
        // device on this network and it is not one.
        if peer.isTicketOnly { return L.t("peer_state_ready_internet") }
        return L.t(isPaired ? "peer_state_ready_paired" : "peer_state_ready")
    }
}

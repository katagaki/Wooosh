import CoreImage.CIFilterBuiltins
import SwiftUI

/// Renders a `wooosh-pair:` or `wooosh-net:` payload as a QR code (CoreImage
/// CIQRCodeGenerator).
struct QRCodeView: View {
    let payload: String
    /// Which kind of code this is, for VoiceOver. The two payloads look
    /// identical on screen but mean different things, so the label must say
    /// which one is being shown.
    var accessibilityKey = "pairing_qr_a11y"

    var body: some View {
        if let image = Self.qrImage(for: payload) {
            Image(decorative: image, scale: 1)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
                .accessibilityLabel(L.t(accessibilityKey))
        } else {
            ContentUnavailableView("Couldn't generate code",
                                   systemImage: "qrcode")
        }
    }

    private static func qrImage(for payload: String) -> CGImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(payload.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else { return nil }
        let scaled = output.transformed(by: CGAffineTransform(scaleX: 10, y: 10))
        return CIContext().createCGImage(scaled, from: scaled.extent)
    }
}

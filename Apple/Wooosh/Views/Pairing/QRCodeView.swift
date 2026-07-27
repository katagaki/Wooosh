import CoreImage.CIFilterBuiltins
import SwiftUI

struct QRCodeView: View {
    let payload: String
    /// The two payloads look identical on screen, so VoiceOver must say which is shown.
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

import CoreImage.CIFilterBuiltins
import SwiftUI

/// Renders a `wooosh-pair:` payload as a QR code (CoreImage CIQRCodeGenerator).
struct QRCodeView: View {
    let payload: String

    var body: some View {
        if let image = Self.qrImage(for: payload) {
            Image(decorative: image, scale: 1)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
                .accessibilityLabel(L.t("pairing_qr_a11y"))
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

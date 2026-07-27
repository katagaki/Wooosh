#if os(iOS)
import AVFoundation
import SwiftUI
import UIKit

/// Camera QR scanner for the pairing payload (AVCaptureMetadataOutput).
/// Requires NSCameraUsageDescription.
struct QRScannerView: UIViewControllerRepresentable {
    let onScan: (String) -> Void

    func makeUIViewController(context: Context) -> ScannerViewController {
        let controller = ScannerViewController()
        controller.onScan = onScan
        return controller
    }

    func updateUIViewController(_ controller: ScannerViewController, context: Context) {}

    final class ScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
        var onScan: ((String) -> Void)?

        private let session = AVCaptureSession()
        private let sessionQueue = DispatchQueue(label: "com.tsubuzaki.Wooosh.qrscan")
        private var previewLayer: AVCaptureVideoPreviewLayer?
        private var captureDevice: AVCaptureDevice?
        private var didDeliver = false
        private var focusIndicator: UIView?

        override func viewDidLoad() {
            super.viewDidLoad()
            view.backgroundColor = .black
            let tap = UITapGestureRecognizer(target: self, action: #selector(handleTap(_:)))
            view.addGestureRecognizer(tap)
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                guard granted else { return }
                self?.sessionQueue.async { self?.configureSession() }
            }
        }

        private func configureSession() {
            guard let device = AVCaptureDevice.default(for: .video),
                  let input = try? AVCaptureDeviceInput(device: device),
                  session.canAddInput(input) else { return }
            session.beginConfiguration()
            session.addInput(input)
            let output = AVCaptureMetadataOutput()
            guard session.canAddOutput(output) else {
                session.commitConfiguration()
                return
            }
            session.addOutput(output)
            output.setMetadataObjectsDelegate(self, queue: .main)
            output.metadataObjectTypes = [.qr]
            session.commitConfiguration()
            session.startRunning()
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                let layer = AVCaptureVideoPreviewLayer(session: self.session)
                layer.videoGravity = .resizeAspectFill
                layer.frame = self.view.bounds
                self.view.layer.addSublayer(layer)
                self.previewLayer = layer
                self.captureDevice = device
            }
        }

        override func viewDidLayoutSubviews() {
            super.viewDidLayoutSubviews()
            previewLayer?.frame = view.bounds
        }

        override func viewWillDisappear(_ animated: Bool) {
            super.viewWillDisappear(animated)
            sessionQueue.async { [session] in
                if session.isRunning { session.stopRunning() }
            }
        }

        // MARK: - Tap to focus

        /// A pairing QR is often small and close-up, which is exactly where
        /// continuous autofocus hunts. Tapping the viewfinder pins focus and
        /// exposure to that point.
        @objc private func handleTap(_ recognizer: UITapGestureRecognizer) {
            let point = recognizer.location(in: view)
            guard let previewLayer, let device = captureDevice else { return }
            // Layer point → the device's own normalized coordinate space; doing
            // this by hand breaks under `.resizeAspectFill` cropping.
            let focusPoint = previewLayer.captureDevicePointConverted(fromLayerPoint: point)
            showFocusIndicator(at: point)
            sessionQueue.async {
                do {
                    try device.lockForConfiguration()
                    defer { device.unlockForConfiguration() }
                    if device.isFocusPointOfInterestSupported {
                        device.focusPointOfInterest = focusPoint
                        if device.isFocusModeSupported(.autoFocus) {
                            device.focusMode = .autoFocus
                        }
                    }
                    if device.isExposurePointOfInterestSupported {
                        device.exposurePointOfInterest = focusPoint
                        if device.isExposureModeSupported(.autoExpose) {
                            device.exposureMode = .autoExpose
                        }
                    }
                } catch {
                    // Focus is a nicety; a device that won't lock just keeps
                    // its continuous autofocus.
                }
            }
        }

        private func showFocusIndicator(at point: CGPoint) {
            focusIndicator?.removeFromSuperview()
            let size: CGFloat = 68
            let indicator = UIView(frame: CGRect(x: point.x - size / 2, y: point.y - size / 2,
                                                 width: size, height: size))
            indicator.layer.borderColor = UIColor.systemYellow.cgColor
            indicator.layer.borderWidth = 1.5
            indicator.layer.cornerRadius = 8
            indicator.isUserInteractionEnabled = false
            view.addSubview(indicator)
            focusIndicator = indicator
            // Respect Reduce Motion: no scale animation, just a fade.
            let animated = !UIAccessibility.isReduceMotionEnabled
            UIView.animate(withDuration: animated ? 0.5 : 0, delay: animated ? 0.35 : 0.6) {
                indicator.alpha = 0
            } completion: { _ in
                indicator.removeFromSuperview()
                if self.focusIndicator === indicator { self.focusIndicator = nil }
            }
        }

        func metadataOutput(_ output: AVCaptureMetadataOutput,
                            didOutput metadataObjects: [AVMetadataObject],
                            from connection: AVCaptureConnection) {
            guard !didDeliver,
                  let object = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
                  object.type == .qr,
                  let payload = object.stringValue else { return }
            didDeliver = true
            // Acknowledged here rather than by whatever the payload triggers:
            // parsing, dialling and hole punching each take long enough that a
            // viewfinder which merely stopped moving reads as a jam. Neutral
            // impact, not `.success` — the code may still turn out to be stale.
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            sessionQueue.async { [session] in
                if session.isRunning { session.stopRunning() }
            }
            onScan?(payload)
        }
    }
}
#endif

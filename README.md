# Wooosh

Wooosh sends files directly between nearby devices on the same network. Transfers are encrypted end to end and go straight from one device to the other, with no accounts, no servers, and nothing stored in the cloud.

Devices find each other automatically. Photos and videos land in your photo library on iOS, and everything else goes to Downloads. Apps for iOS, macOS, and Android share one Rust core that handles pairing, encryption, and transfers.

## Requirements

- macOS with Xcode 26 or later (the apps target iOS 26 and macOS 26)
- Android Studio with an Android SDK and NDK
- Rust, via `brew install rustup && rustup-init`

## Setup

1. Build the Rust core and its bindings. This is required before the Apple app will build, because the compiled framework is not checked in.

   ```
   export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
   ./Core/build-bindings.sh
   ```

2. Open `Apple/Wooosh.xcodeproj` in Xcode, select your own signing team under Signing and Capabilities, then build and run.

3. Open the `Android` folder in Android Studio, let it sync, then build and run.

To run the tests for the core:

```
cd Core && cargo test --release
```

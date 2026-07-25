#!/bin/bash
# Build wooosh-core FFI deliverables:
#   bindings/swift  + dist/WoooshCore.xcframework
#       macos-arm64 · ios-arm64 · ios-arm64_x86_64-simulator (fat)
#   bindings/kotlin + dist/jniLibs (arm64-v8a, x86_64)
set -euo pipefail
cd "$(dirname "$0")"

export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
NDK_ROOT="${ANDROID_NDK_HOME:-$(ls -d "$HOME"/Library/Android/sdk/ndk/* 2>/dev/null | sort -V | tail -1)}"

APPLE_TARGETS=(aarch64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios)
echo "==> rust targets"
for t in "${APPLE_TARGETS[@]}"; do
    rustup target add "$t" >/dev/null
done

echo "==> host release build"
cargo build --release -p wooosh-core

echo "==> uniffi bindings (swift + kotlin)"
rm -rf bindings/swift bindings/kotlin
cargo run --release -p uniffi-bindgen -- generate \
    --library target/release/libwooosh_core.dylib \
    --language swift --out-dir bindings/swift
cargo run --release -p uniffi-bindgen -- generate \
    --library target/release/libwooosh_core.dylib \
    --language kotlin --out-dir bindings/kotlin

echo "==> apple static libs"
cargo build --release -p wooosh-core --target aarch64-apple-darwin
IPHONEOS_DEPLOYMENT_TARGET=15.0 cargo build --release -p wooosh-core --target aarch64-apple-ios
IPHONEOS_DEPLOYMENT_TARGET=15.0 cargo build --release -p wooosh-core --target aarch64-apple-ios-sim
# Intel simulator slice: without it `-destination generic/platform=iOS Simulator`
# fails to link on Intel Macs (and forces an EXCLUDED_ARCHS workaround in the
# Xcode project). x86_64-apple-ios *is* the Intel simulator target.
IPHONEOS_DEPLOYMENT_TARGET=15.0 cargo build --release -p wooosh-core --target x86_64-apple-ios

echo "==> fat simulator lib (arm64 + x86_64)"
# One xcframework slice must carry both simulator archs; two separate
# -library arguments for the same platform+variant are rejected.
SIMFAT=$(mktemp -d)/ios-simulator
mkdir -p "$SIMFAT"
lipo -create \
    target/aarch64-apple-ios-sim/release/libwooosh_core.a \
    target/x86_64-apple-ios/release/libwooosh_core.a \
    -output "$SIMFAT/libwooosh_core.a"
lipo -info "$SIMFAT/libwooosh_core.a"

echo "==> xcframework"
# uniffi swift output: wooosh_core.swift + wooosh_coreFFI.h + wooosh_coreFFI.modulemap
HDRS=$(mktemp -d)/headers
mkdir -p "$HDRS"
cp bindings/swift/wooosh_coreFFI.h "$HDRS/"
cp bindings/swift/wooosh_coreFFI.modulemap "$HDRS/module.modulemap"
rm -rf dist/WoooshCore.xcframework
mkdir -p dist
xcodebuild -create-xcframework \
    -library target/aarch64-apple-darwin/release/libwooosh_core.a -headers "$HDRS" \
    -library target/aarch64-apple-ios/release/libwooosh_core.a -headers "$HDRS" \
    -library "$SIMFAT/libwooosh_core.a" -headers "$HDRS" \
    -output dist/WoooshCore.xcframework
echo "==> xcframework slices"
for lib in dist/WoooshCore.xcframework/*/libwooosh_core.a; do
    printf '    %s: ' "$(dirname "$lib" | xargs basename)"
    lipo -info "$lib" | sed 's/.*are: //; s/.*is architecture: //'
done

echo "==> android jniLibs"
if [ -n "$NDK_ROOT" ] && [ -d "$NDK_ROOT" ]; then
    export ANDROID_NDK_HOME="$NDK_ROOT"
    rm -rf dist/jniLibs
    cargo ndk -t arm64-v8a -t x86_64 -o dist/jniLibs build --release -p wooosh-core
    # cargo-ndk copies every .so it finds, including cdylib artifacts of
    # dependencies (iroh ships one). wooosh_core links them statically — it
    # NEEDs only libdl/libm/libc — so shipping those would add megabytes of
    # dead weight to every APK.
    find dist/jniLibs -name '*.so' ! -name 'libwooosh_core.so' -delete
else
    echo "!! no Android NDK found — skipping jniLibs" >&2
fi

echo "==> done"
find dist -maxdepth 3 -print

#!/bin/sh
# Xcode Cloud: runs after the repository is cloned, before dependency resolution.
#
# Core/dist/ is gitignored, so the freshly cloned tree has no
# WoooshCore.xcframework and the Apple target cannot link. Build it here.
#
# build-bindings.sh skips the Android lane by itself when no NDK is present,
# which is always the case on Xcode Cloud, so it needs no CI-specific flags.
set -eu

# Xcode Cloud sets CI_PRIMARY_REPOSITORY_PATH. Fall back to the path relative to
# this script so the same file can be run by hand to reproduce a CI failure.
REPO="${CI_PRIMARY_REPOSITORY_PATH:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$REPO"

echo "==> repository: $REPO"

# Rust is not preinstalled on Xcode Cloud images. The minimal profile skips
# docs and clippy, which shaves a noticeable amount off a cold build.
if ! command -v cargo >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    echo "==> installing rust"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path
fi
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
export PATH="$HOME/.cargo/bin:$PATH"

cargo --version
rustc --version

echo "==> building the rust core and FFI artifacts"
./Core/build-bindings.sh

# Fail loudly here rather than letting the Xcode build fail later with an
# opaque linker error about missing symbols.
XCF="Core/dist/WoooshCore.xcframework"
if [ ! -d "$XCF" ]; then
    echo "!! $XCF was not produced; the Apple target cannot link" >&2
    exit 1
fi

echo "==> ready"
ls "$XCF"

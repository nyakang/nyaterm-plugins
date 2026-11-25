#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."
test "$(getconf GNU_LIBC_VERSION)" = 'glibc 2.31'
test "$(uname -m)" = x86_64
export CARGO_TARGET_DIR="$PWD/target/focal"
export NYATERM_ANDROID_DEPS_DIR="$CARGO_TARGET_DIR/android-deps"
export NYATERM_SPICE_DEPS_DIR="$CARGO_TARGET_DIR/spice-deps"
export CARGO_BUILD_JOBS=4
target=x86_64-unknown-linux-gnu
python3 scripts/release/components.py build --component "$COMPONENT" --target "$target"
cargo test --locked --release -p nyaterm-native-protocol --target "$target"
python3 scripts/release/components.py test --component "$COMPONENT" --target "$target"
python3 scripts/release/components.py package --component "$COMPONENT" --target "$target" --runtime glibc-2.31 --host-version "$HOST_VERSION" --archive "dist/$COMPONENT-$target-glibc-2.31.zip"

# Official native components

Independent Rust workspace for Android, Builtin RDP, VNC and SPICE. Each helper has
its own SemVer in Cargo.toml. nyaterm-native-protocol owns bounded wire contracts;
the application pins it by exact Git revision. No crate depends on the app checkout.
Windows Native RDP remains in NyaTerm.

## Development

Run from this directory:

```sh
python scripts/release/prepare_android.py --target x86_64-pc-windows-msvc
# Export FFMPEG_DIR printed by preparation; add its bin directory to PATH on Windows.
cargo test --workspace --all-targets --features nyaterm-android-helper/test-support
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
python -m unittest discover -s scripts/tests
```

The Android native linking acceptance test requires prepared FFmpeg and MSVC.
NYATERM_ANDROID_DEPS_DIR and NYATERM_SPICE_DEPS_DIR can point to external caches.
Never point a production helper at the synthetic test endpoint: that option exists
only with the default-off test-support feature.

```sh
python scripts/release/components.py build --component vnc --target x86_64-pc-windows-msvc
python scripts/release/components.py package --component vnc --target x86_64-pc-windows-msvc --runtime windows-msvc --host-version ">=2.0.8, <3" --archive dist/vnc.zip
```

Android packages own adb, FFmpeg and the embedded scrcpy server. SPICE packages own
its dynamic runtime. Component packaging checks inventory, executable architecture,
macOS deployment version and Linux glibc requirements. Helpers retain decoder panic
containment and server-facing security policy enforcement.

## Release

Dispatch Official native components for one component. The six normal targets and
x64 glibc 2.31 build must succeed. Publication runs on main only, behind the
native-signing environment. Configure TAURI_SIGNING_PRIVATE_KEY,
TAURI_SIGNING_PRIVATE_KEY_PASSWORD, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY and
R2_ENDPOINT. The public key must match the application's native component key.

Assets use native/v1/<component>/<version>/<target>/<runtime>/component.zip.
Versions are immutable. Bump only the affected helper version for a new release.
A signed index envelope is updated only after all asset writes/readbacks succeed;
ETag compare-and-swap rejects concurrent replacement. Existing signed index entries
are preserved. R2 publication is separate from NyaTerm application releases.

IPC versions are independent per component. Bump the affected IPC constant on wire
breaks, publish the protocol commit, then update the application's exact revision.
Host SemVer bounds describe tested compatibility; they do not replace IPC checks.
Do not put credentials in logs or diagnostics. Live device/server and macOS/Linux
acceptance must be performed on their target platforms before release.

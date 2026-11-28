# Official native components

Independent Rust workspace for Android, Builtin RDP, VNC and SPICE. Each helper has
its own SemVer in Cargo.toml. nyaterm-native-protocol owns bounded wire contracts;
the application pins it by exact Git revision. No crate depends on the app checkout.
Windows Native RDP remains in NyaTerm.

## RDP presentation

RDP IPC 13 supports an optional `frame_update_complete` boolean in `ClientHello`
and `ServerHello.capabilities`. Missing fields mean false. The helper advertises
support, but emits `FrameUpdateComplete { session_id, epoch, sequence }` only when
the application opted in. Legacy applications continue receiving the existing
bitmap packets. New applications receiving an older helper's capabilities can
treat each bitmap as an update boundary.

All bitmap regions for a submission precede its completion message on the pipe.
Completion sequences start at one and increase within an epoch. A connection,
reconnection or framebuffer size change starts a new epoch before new pixels;
requesting a full frame keeps the current epoch. Completion flushes the buffered
writer even while more output is queued. Large regions are split into rows that
fit the existing frame-packet byte limit.

The helper honors the saved GFX request at 32-bit color depth. Other color depths
keep bitmap updates without rewriting the saved request. GFX codec selection and
frame acknowledgments stay inside IronRDP; the decoder runs only in the helper.
The legacy bitmap-codec list is independent of GFX negotiation. Setting
`NYATERM_RDP_METRICS=1` adds numeric IPC write/flush and output-queue wait timings
to stderr without logging hosts, accounts, clipboard data or framebuffer pixels.

RDP 1.2.2 supports GNOME remote-login handover through FreeRDP's enhanced
server-redirection header and RDSTLS encrypted-password authentication.
The redirected TLS certificate must exactly match the certificate supplied
by the broker, and configured certificate rejection remains authoritative.
RDSTLS authentication finishes before MCS; encrypted credentials are never
sent as a normal Client Info password. The component uses rustls and retains
IPC 13, relay approval and existing persisted data formats. On November 27,
2026, a Windows client successfully connected to Ubuntu running GNOME Remote
Desktop 50.2 and FreeRDP 3.32.1; RDSTLS handover and desktop frames were verified.

FreeRDP's combined RDSTLS version 1/2 advertisement is accepted while the
helper continues authenticating with version 1. A timeout after activation
reports that the first desktop frame is missing rather than reporting an
unfinished security negotiation.

For connection troubleshooting, set `NYATERM_RDP_PHASE_LOG` to a writable file
before launching the application. The helper appends only allowlisted phase
names, its process ID and elapsed milliseconds; identities, credentials and
protocol payloads are excluded. Logging stops when the file reaches 1 MiB;
remove or rotate it manually before collecting another diagnostic run. A
`dvc_capabilities_response_generated` milestone means the reply was encoded,
not that the server received it.

## RDP clipboard ownership (1.2.3)

IPC 13 adds opt-in `managed_clipboard` fields to ClientHello and server
capabilities. Both default to false. Only negotiated peers exchange the managed
clipboard commands/events. Without client opt-in the helper disables clipboard
redirection and emits a nonfatal update notice; a new application similarly
turns off the ephemeral clipboard setting when connecting to an older helper.
Saved connection settings and backup formats do not change.

The application owns one persistent system clipboard connection, observes local
text/file changes and grants a session an authorization epoch and private cache
root. The helper never accesses the system clipboard. It accepts snapshots only
for the active epoch, returns Started/Ready events tagged with that epoch and
request plus the last observed local clipboard revision, and sends file paths relative to the application-provided cache root.
The application acknowledges remote publications using Observed commands without
echoing their contents; delayed Started notifications cannot rebase onto a newer
local copy. Local snapshot scanning runs on a bounded worker; revocation is not queued
behind a directory scan. Completed file caches survive helper shutdown so a
local paste can finish. The application validates paths, checks clipboard
freshness, publishes content and reports transfer completion. Partial downloads
are cancelled on revocation; late stream responses cannot fail a newer transfer.

Drain access permits an existing remote download to finish after Alt+Tab, but
starts no new copies. Switching sessions or locking the application revokes the
epoch. The application must recheck both epoch and local clipboard contents
before publication, including after cache validation. A missing format response
reports a nonfatal timeout and requires manual reconnect if it never arrives;
reusing the outstanding slot would misidentify a late response.

Validation: Linux protocol/state tests and helper lifecycle tests cover legacy
handshakes, revoked requests, drain access, repeated text/file transitions,
retained caches and stale chunks. Windows/macOS and live Windows RDP clipboard
interoperability require platform smoke testing before component publication.

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

## Release and first deployment

Native components use the manual **Official native components** workflow. Merging
main or creating a tag does not publish. The application discovers releases from
`https://downloads.nyaterm.app/native/v1/index.json`, not GitHub Releases.

Configure the `native-signing` environment with owner review and a deployment
branch policy restricted to `main`. The workflow uses these secret names:

- `TAURI_SIGNING_PRIVATE_KEY_B64`: an outer base64 encoding of the signing key
  accepted by Tauri's `TAURI_SIGNING_PRIVATE_KEY` environment variable, matching
  the application's existing release convention.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: password for that key.
- `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`: credentials for
  the existing `nyaterm` bucket served by `downloads.nyaterm.app`.

Repository secrets may supply R2 configuration; keep signing secrets in the
protected environment. Never paste secrets into issues, logs, or workflow inputs.
The signing key must match `scripts/release/updater-public-key.txt` and the
application's native component public key. Preflight signs and verifies a local
challenge before platform builds; it does not publish objects.

### Routine component update

1. Change the affected helper and run its tests plus packaging tests.
2. Increment only its `crates/nyaterm-<component>-helper/Cargo.toml` version and
   update `Cargo.lock`. Merge the reviewed change into main.
3. Dispatch **Official native components** with `publish=false`, the component
   and a tested application SemVer range (initial default `>=2.0.8, <3`). All six
   normal targets and x64 glibc 2.31 must pass. Unsigned build artifacts cannot be
   imported by production NyaTerm.
4. Dispatch the same commit from main with `publish=true`. Approve protected
   environment jobs after reviewing the source and results. The workflow checks
   configuration, builds, signs and preserves seven packages, then publishes.
5. Download the publication summary artifact and run:

   ```sh
   python scripts/release/verify_public.py --component vnc --version 1.0.0
   ```

For first deployment publish VNC, RDP, SPICE, then Android at `1.0.0`, provided
those identities have never been published. Run the verifier without `--component`
to check all 28 archives. The verifier requires minisign. Allow the index's
60-second CDN cache to expire before checking a just-published release.

### Failures and recovery

The build, signing and publishing jobs are separate. Signed packages are retained
as `signed-native-<component>` artifacts for 30 days. The publishing job records
source commits, compatibility, archive hashes, verified object keys and index
status in `publication-<component>-<attempt>` even when uploading fails.

If only publication fails, **re-run failed jobs**, preserving the signed artifact
from the successful signing job. Identical previously uploaded bytes are accepted;
different content at the same version is rejected. Do not rebuild/re-sign that
version or re-run all jobs after any asset has been uploaded. If the original
signed artifact is unavailable, increment the helper version. Index conflicts
require a fresh publication attempt, which reads and verifies the current index.
Publish components sequentially; the shared index job is serialized.

If upload and index publication succeeded but public verification failed, the
read-only **Verify published native components** workflow can recheck the existing
release after fixing CDN/client issues. Select the component (or `all`) and version;
it verifies signatures and every platform archive and preserves a verification
report without credentials, rebuilding, signing, or writing to R2.

Assets live at `native/v1/<component>/<version>/<target>/<runtime>/component.zip`.
They are immutable. The signed index is replaced with an ETag condition only after
all asset writes/readbacks succeed; previous releases remain in the index. Failed
publication may leave unreferenced immutable assets but cannot advertise a partial
platform set. R2 publication is independent of application releases.

### Application installation and compatibility

In NyaTerm's Plugins page, install a native component or import an official signed
ZIP. First use of an unavailable Android/RDP/VNC/SPICE backend also offers download
and installation. Check for updates from the component menu, then explicitly apply
the available update; no background automatic replacement occurs. The menu also
supports rollback, disable and uninstall. An update leaves existing sessions on
their leased version; new sessions use the new selection. Compatible installations
survive application upgrades. Windows Native RDP remains application-owned.

Helper versions are independent of application versions. Selection checks host
SemVer bounds, target/runtime and the component's IPC version. Bump the affected
IPC constant on incompatible wire changes, publish that protocol commit, then
update the application's exact Git revision. Decoder fixes with unchanged contracts
do not require an application release. The first rollout requires users to install
a NyaTerm build containing the independent-component client.

Validate download/install/launch using an isolated application data directory.
Exercise updates and rollback with controlled test versions, not fake production
releases. Real Android and remote-server sessions and Linux/macOS runtime checks
remain separate acceptance steps; record unavailable devices/platforms explicitly.

Host acceptance commands (run in the NyaTerm repository):

```powershell
$env:NYATERM_TEST_NATIVE_COMPONENT = 'vnc' # repeat for rdp, spice, android
cargo test -p nyaterm-plugin-host --test native_acceptance public_release_installs_and_reopens_in_isolated_registry -- --ignored
# Local signed test ZIPs only; these versions must never be published.
$env:NYATERM_NATIVE_TEST_OLD = 'absolute/path/to/old.zip'
$env:NYATERM_NATIVE_TEST_NEW = 'absolute/path/to/new.zip'
cargo test -p nyaterm-plugin-host --test native_acceptance signed_import_update_rollback_and_failed_install_preserve_selection -- --ignored
```

These tests create isolated registries and reject helper path overrides. They
validate installation and selection, not real-device or remote-server sessions.
The publication job also verifies the public downloads, with bounded retries for
index cache propagation; its result is included in the publication summary.

## X11 component

Windows x64 X11 packages contain `nyaterm-x11-helper.exe` and a pinned VcXsrv
runtime built by `scripts/release/prepare_x11.py`. Its source fork is
`nyakang/vcxsrv`, branch `nyaterm`; `SOURCE.json` and `NYATERM.md` identify the
exact source and loopback-listener patch. The component owns no framebuffer IPC.
It starts one authenticated server per SSH session and defaults clipboard off.
The package includes misc/scalable fonts; upstream's optional 75/100-DPI bitmap
encoding collections are omitted to fit the existing 4096-file manifest limit.

The X11 control pipe is bounded and versioned. Each server has a private
authority file and a kill-on-close Job Object; EOF, Stop and helper death clean
up its process tree. Runtime validation uses the actual staged package:

```sh
python scripts/release/verify_x11.py target/x11-smoke
```

The X11 validation workflow builds on Windows and verifies authentication,
loopback listeners, simultaneous independent instances, ordinary shutdown and
forced helper death. Desktop rendering, clipboard opt-in, DPI and multi-monitor
behavior also require an interactive Windows smoke test before release.

X11 publication uses `native/v1/x11/index.json`. Never add an `x11` record to
`native/v1/index.json`: old NyaTerm versions cannot deserialize that component.
The existing protected manual publication workflow supports X11 after the
Windows package checks pass. ARM64, macOS and Linux X11 packages are not published.

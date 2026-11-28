use std::fmt;
use std::sync::Arc;

use crate::secret::SecretString;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 13;
/// VNC evolves independently from the RDP contract.
pub const VNC_PROTOCOL_VERSION: u32 = 13;

/// Maximum UTF-8 payload accepted for a single committed-text event.
///
/// This intentionally leaves ample room below the control-packet envelope for
/// JSON framing and other events in the same input batch.
pub const MAX_COMMITTED_TEXT_BYTES: usize = 64 * 1024;

/// Desktop extent the protocol will negotiate, in pixels. Shared by
/// `RdpDisplayMetrics::clamped` and the connect/resize validation so a request
/// the application is allowed to make is never rejected at the session layer.
pub const MIN_WIDTH: u32 = 200;
pub const MIN_HEIGHT: u32 = 200;
pub const MAX_WIDTH: u32 = 8192;
pub const MAX_HEIGHT: u32 = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CommittedTextError {
    #[error("committed text exceeds the {MAX_COMMITTED_TEXT_BYTES}-byte limit")]
    TooLong,
    #[error("committed text contains NUL")]
    ContainsNul,
    #[error("committed text contains a control character that must be sent as a key event")]
    ContainsControl,
}

/// Validate an entire committed-text payload before it is put on the wire.
///
/// Committed text is Unicode text produced by an IME or text service. Control
/// characters are deliberately excluded: Enter, Tab, Escape, and similar input
/// must retain their protocol-specific key semantics instead of being smuggled
/// through a text event.
pub fn validate_committed_text(text: &str) -> Result<(), CommittedTextError> {
    if text.len() > MAX_COMMITTED_TEXT_BYTES {
        return Err(CommittedTextError::TooLong);
    }
    if text.contains('\0') {
        return Err(CommittedTextError::ContainsNul);
    }
    if text.chars().any(char::is_control) {
        return Err(CommittedTextError::ContainsControl);
    }
    Ok(())
}

/// Static features implemented by an RDP helper for this IPC version.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpServerCapabilities {
    #[serde(default)]
    pub managed_clipboard: bool,
    pub committed_unicode_text: bool,
    pub secure_attention: bool,
    /// Ordered desktop submission boundaries, emitted only when the client opts in.
    #[serde(default)]
    pub frame_update_complete: bool,
    /// Target-specific certificate and route coordination, enabled only by client opt-in.
    #[serde(default)]
    pub server_redirection: bool,
}

/// Static features implemented by a VNC helper for this IPC version.
///
/// Secure Attention is intentionally absent: VNC has no SAS wire operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncServerCapabilities {
    pub committed_unicode_keysyms: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpBackend {
    #[default]
    #[serde(alias = "auto", alias = "internal")]
    Builtin,
    #[serde(alias = "active-x")]
    Windows,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpSessionConfig {
    #[serde(default)]
    pub relay: Option<crate::relay::RelayEndpoint>,
    #[serde(default)]
    pub backend: RdpBackend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_authentication: Option<String>,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub domain: String,
    pub password: Option<SecretString>,
    pub use_nla: bool,
    pub certificate_policy: RdpCertificatePolicy,
    /// Request an administrative session; assignment remains server-controlled.
    #[serde(default)]
    pub administrative_session: bool,
    pub display: RdpDisplayConfig,
    pub clipboard: RdpClipboardConfig,
    /// Local resource redirection. Default keeps devices off and clipboard on.
    #[serde(default)]
    pub redirection: RdpRedirectionConfig,
    pub reconnect: RdpReconnectConfig,
}

impl RdpSessionConfig {
    /// Apply backend policy to an ephemeral launch configuration, never stored data.
    /// Clipboard follows the new switch; legacy modes remain serializable for
    /// saved sessions, backups, and sync compatibility.
    /// Selecting Windows uses system authentication and drops WebAuthn, which
    /// mstscax has no property for; unknown auth modes remain errors.
    pub fn normalize_backend_settings(&mut self) {
        self.clipboard.mode = if self.redirection.clipboard {
            RdpClipboardMode::TextAndFiles
        } else {
            RdpClipboardMode::Disabled
        };
        if self.backend == RdpBackend::Windows {
            self.windows_authentication
                .get_or_insert_with(|| "system".into());
            self.redirection.webauthn = false;
        }
    }
}

impl fmt::Debug for RdpSessionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RdpSessionConfig")
            .field("backend", &self.backend)
            .field("name", &self.name)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("domain", &self.domain)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("use_nla", &self.use_nla)
            .field("certificate_policy", &self.certificate_policy)
            .field("administrative_session", &self.administrative_session)
            .field("display", &self.display)
            .field("clipboard", &self.clipboard)
            .field("redirection", &self.redirection)
            .field("reconnect", &self.reconnect)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpCertificatePolicy {
    #[default]
    Prompt,
    TrustOnFirstUse,
    Strict,
    #[serde(alias = "accept-temporarily")]
    Insecure,
    #[serde(alias = "reject_changed")]
    RejectChanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpDisplayConfig {
    pub mode: RdpDisplayMode,
    pub width: u32,
    pub height: u32,
    /// Desktop scale factor in percent (100..=500) negotiated with the initial
    /// desktop size, or 0 to leave the choice to the server.
    ///
    /// This is ephemeral client display information, like
    /// [`RdpDisplayMetrics`]: it is filled in from the panel that hosts the
    /// session and is never taken from a saved connection. Asking for the scale
    /// in the same handshake as the size is what keeps a fit-window session
    /// from changing resolution again right after it connects.
    #[serde(default)]
    pub desktop_scale_factor: u32,
    pub color_depth: u8,
    /// RDPEGFX surface updates. See `nyaterm_core::RdpDisplaySettings`.
    #[serde(default)]
    pub graphics_pipeline: bool,
    #[serde(default = "rdp_default_true")]
    pub display_wallpaper: bool,
    #[serde(default = "rdp_default_true")]
    pub display_themes: bool,
    #[serde(default = "rdp_default_true")]
    pub enable_font_smoothing: bool,
    #[serde(default)]
    pub enable_desktop_composition: bool,
    /// Inverted: true asks the server to turn the effect off.
    #[serde(default = "rdp_default_true")]
    pub disable_full_window_drag: bool,
    #[serde(default = "rdp_default_true")]
    pub disable_menu_animations: bool,
    #[serde(default)]
    pub disable_cursor_shadow: bool,
    #[serde(default)]
    pub disable_cursor_blinking: bool,
}

/// Ephemeral client display information sent to the helper. This is not part
/// of persisted connection configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpDisplayMetrics {
    pub width: u32,
    pub height: u32,
    pub desktop_scale_factor: u32,
    pub physical_size_mm: Option<(u32, u32)>,
}

impl RdpDisplayMetrics {
    /// Bring a panel-derived request into the range the protocol accepts.
    ///
    /// Even boundaries because the legacy desktop size fields are `u16` pixels
    /// and the server rounds the framebuffer to them; a request that is not
    /// even comes back as a different size, which looks like a resize stutter.
    pub fn clamped(self) -> Self {
        Self {
            width: self.width.clamp(MIN_WIDTH, MAX_WIDTH) & !1,
            height: self.height.clamp(MIN_HEIGHT, MAX_HEIGHT) & !1,
            desktop_scale_factor: self.desktop_scale_factor.clamp(100, 500),
            physical_size_mm: self.physical_size_mm,
        }
    }
}

fn rdp_default_true() -> bool {
    true
}

impl Default for RdpDisplayConfig {
    fn default() -> Self {
        Self {
            mode: RdpDisplayMode::FitWindow,
            width: 1920,
            height: 1080,
            desktop_scale_factor: 0,
            color_depth: 32,
            graphics_pipeline: false,
            display_wallpaper: true,
            display_themes: true,
            enable_font_smoothing: true,
            enable_desktop_composition: false,
            disable_full_window_drag: true,
            disable_menu_animations: true,
            disable_cursor_shadow: false,
            disable_cursor_blinking: false,
        }
    }
}

/// Build the helper's display configuration from a saved connection.
///
/// Both launch paths (opening a connection and restoring one at startup) go
/// through here, so a setting can never be honoured when opening a tab and
/// ignored after a restart.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpDisplayMode {
    #[default]
    FitWindow,
    Fixed,
}

impl RdpDisplayConfig {
    /// Ask the server for exactly the desktop the hosting panel will draw into.
    ///
    /// A fit-window session that connects at the saved size and is resized to
    /// the panel afterwards shows the desktop twice at two different scales in
    /// the time it takes to connect; the panel is measurable before the connect
    /// is issued, so the first negotiation should already use it.
    pub fn fit_to_panel(&mut self, metrics: RdpDisplayMetrics) {
        let metrics = metrics.clamped();
        self.width = metrics.width;
        self.height = metrics.height;
        self.desktop_scale_factor = metrics.desktop_scale_factor;
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpClipboardConfig {
    pub mode: RdpClipboardMode,
}

impl Default for RdpClipboardConfig {
    fn default() -> Self {
        Self {
            mode: RdpClipboardMode::TextAndFiles,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpClipboardMode {
    Disabled,
    TextOnly,
    #[default]
    TextAndFiles,
}

/// Playback destination selected in the Local resources panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RdpAudioMode {
    /// Do not play remote audio anywhere (the historical client behavior).
    #[default]
    NoAudio,
    /// Play remote audio on this computer.
    Local,
    /// Leave audio playing on the remote computer.
    Remote,
}

/// Parse an editor/CLI audio mode, falling back to `no_audio` for unknown values.
pub fn parse_rdp_audio_mode(value: &str) -> RdpAudioMode {
    match value {
        "local" | "play_on_local" | "redirect" => RdpAudioMode::Local,
        "remote" | "play_on_remote" | "resource_only" => RdpAudioMode::Remote,
        _ => RdpAudioMode::NoAudio,
    }
}

/// Sentinel for "redirect every local drive", matching mstsc's `drivestoredirect:s:*`.
pub const RDP_ALL_DRIVES: &str = "*";

/// Default drive selection for newly created connections.
pub fn default_rdp_drive_letters() -> String {
    RDP_ALL_DRIVES.to_string()
}

/// Parse the persisted drive selection into distinct uppercase letters.
/// `"*"` expands to every letter; an empty string selects nothing.
pub fn parse_rdp_drive_letters(spec: &str) -> Vec<char> {
    if spec.trim() == RDP_ALL_DRIVES {
        return ('A'..='Z').collect();
    }
    let mut letters: Vec<char> = spec
        .chars()
        .filter_map(|letter| {
            let upper = letter.to_ascii_uppercase();
            upper.is_ascii_uppercase().then_some(upper)
        })
        .collect();
    letters.sort_unstable();
    letters.dedup();
    letters
}

/// Toggle a single drive letter, preserving the wildcard only when all 26
/// letters remain selected.
pub fn toggle_rdp_drive_letter(spec: &str, letter: char) -> String {
    let letter = letter.to_ascii_uppercase();
    if !letter.is_ascii_uppercase() {
        return spec.to_string();
    }
    let mut letters = parse_rdp_drive_letters(spec);
    match letters.iter().position(|value| *value == letter) {
        Some(index) => {
            letters.remove(index);
        }
        None => letters.push(letter),
    }
    if letters.len() == 26 {
        RDP_ALL_DRIVES.to_string()
    } else {
        letters.into_iter().collect()
    }
}

/// Local resource redirection carried to both RDP backends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpRedirectionConfig {
    #[serde(default)]
    pub drives: bool,
    /// See [`parse_rdp_drive_letters`] for the accepted values.
    #[serde(default = "default_rdp_drive_letters")]
    pub drive_letters: String,
    /// Optional local directory announced as an extra RDPDR share. Empty
    /// disables it on Windows; on Linux/macOS an empty value falls back to
    /// the home directory so the single configured share keeps working.
    #[serde(default)]
    pub custom_drive_path: String,
    #[serde(default)]
    pub printers: bool,
    #[serde(default)]
    pub ports: bool,
    #[serde(default)]
    pub smartcards: bool,
    #[serde(default)]
    pub microphone: bool,
    /// Not supported by mstscax; normalized off for the Windows backend.
    #[serde(default)]
    pub webauthn: bool,
    /// Mirrors `RdpClipboardConfig::mode` and is on by default.
    #[serde(default = "default_redirection_true")]
    pub clipboard: bool,
    #[serde(default)]
    pub audio: RdpAudioMode,
}

fn default_redirection_true() -> bool {
    true
}

impl Default for RdpRedirectionConfig {
    fn default() -> Self {
        Self {
            drives: false,
            drive_letters: default_rdp_drive_letters(),
            custom_drive_path: String::new(),
            printers: false,
            ports: false,
            smartcards: false,
            microphone: false,
            webauthn: false,
            clipboard: true,
            audio: RdpAudioMode::NoAudio,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RdpClipboardTransferStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpClipboardTransferProgress {
    pub id: String,
    pub name: String,
    pub status: RdpClipboardTransferStatus,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub total_files: u64,
    pub completed_files: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpReconnectConfig {
    pub enabled: bool,
    pub max_attempts: u32,
}

impl Default for RdpReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RdpInputEvent {
    KeyDown {
        scan_code: u16,
        extended: bool,
        repeat: bool,
    },
    KeyUp {
        scan_code: u16,
        extended: bool,
        repeat: bool,
    },
    Unicode {
        text: String,
    },
    Pointer(RemotePointerEvent),
    ReleaseAllInputs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemotePoint {
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemotePointerButton {
    Left,
    Middle,
    Right,
    X1,
    X2,
}

/// Compatibility name retained for desktop call sites while pointer input is
/// moved onto the protocol-neutral model.
pub type RdpPointerButton = RemotePointerButton;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteWheelAxis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemotePointerEvent {
    Move {
        position: RemotePoint,
    },
    Button {
        position: RemotePoint,
        button: RemotePointerButton,
        pressed: bool,
    },
    Wheel {
        position: RemotePoint,
        axis: RemoteWheelAxis,
        rotation_units: i16,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PixelFormat {
    Bgra8,
    Rgba8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteFrameEvent {
    Reset {
        epoch: u64,
        width: u32,
        height: u32,
    },
    Bitmap {
        epoch: u64,
        full: bool,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        stride: u32,
        format: PixelFormat,
        pixels: crate::pixel_memory::PixelBuffer,
    },
}

/// Source-compatible alias for integrations migrating to the protocol-neutral name.
pub type RdpFrameEvent = RemoteFrameEvent;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorShape {
    pub shape_id: u64,
    pub width: u32,
    pub height: u32,
    pub hotspot: RemotePoint,
    pub pixels: crate::pixel_memory::PixelBuffer,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CursorPosition {
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorVisibility {
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteCursorEvent {
    Shape(CursorShape),
    Position(CursorPosition),
    Visibility(CursorVisibility),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RdpSessionState {
    Idle,
    Connecting,
    Connected,
    Reconnecting,
    Disconnecting,
    Disconnected,
    Failed(RdpError),
    #[deprecated(note = "use Connected")]
    Active,
    #[deprecated(note = "use Disconnected")]
    Closed,
}

/// A target to authorize before the helper opens its next connection.
/// See CONTRACT.rdp-redirection.md for correlation and compatibility rules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpRedirectRequest {
    pub session_id: String,
    pub request_id: String,
    pub generation: u64,
    pub hop: u32,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub connect_hosts: Vec<String>,
}

impl RdpRedirectRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.session_id.is_empty()
            || self.session_id.len() > 128
            || self.request_id.is_empty()
            || self.request_id.len() > 128
            || self.generation == 0
            || !(1..=8).contains(&self.hop)
            || self.host.is_empty()
            || self.host.len() > 253
            || self
                .host
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
            || self.port == 0
            || self.connect_hosts.len() > 16
            || self
                .connect_hosts
                .iter()
                .any(|host| !valid_redirect_host(host))
            || !valid_redirect_host(&self.host)
        {
            return Err("invalid RDP redirection target");
        }
        Ok(())
    }
}

fn valid_redirect_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && (host.parse::<std::net::IpAddr>().is_ok()
            || host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RdpRedirectResponse {
    Ready {
        relay: Option<crate::relay::RelayEndpoint>,
    },
    Rejected {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpCertificateRequest {
    pub request_id: String,
    pub host: String,
    pub port: u16,
    pub sha256_fingerprint: String,
    pub subject: Option<String>,
    pub issuer: Option<String>,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RdpCertificateResponse {
    Reject,
    TrustOnce,
    TrustAndRemember,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RdpCapability {
    DynamicResizeUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RdpErrorKind {
    ResourceExhausted,
    Authentication,
    CertificateRejected,
    Timeout,
    ConnectionRefused,
    Tls,
    Transport,
    /// The live session ended: the server stated a reason, or the session broke in a way
    /// the helper could not attribute to the transport.
    ///
    /// Deliberately not auto-reconnectable. A server that ended a session for a reason --
    /// most often a takeover by another connection -- ends the replacement the same way,
    /// so two NyaTerm clients chasing each other leaves neither usable.
    Session,
    Clipboard,
    Negotiation,
    HelperMissing,
    HelperCrashed,
    Ipc,
    Protocol,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct RdpError {
    pub kind: RdpErrorKind,
    pub message: String,
}

impl RdpError {
    pub fn new(kind: RdpErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RdpRuntimeEvent {
    State {
        session_id: String,
        state: RdpSessionState,
        message: Option<String>,
    },
    Frame {
        session_id: String,
        event: RemoteFrameEvent,
    },
    Cursor {
        session_id: String,
        event: RemoteCursorEvent,
    },
    ManagedClipboard {
        session_id: String,
        event: crate::clipboard::ClipboardEvent,
    },
    Clipboard {
        session_id: String,
        text: String,
        generation: u64,
    },
    ClipboardTransfer {
        session_id: String,
        progress: RdpClipboardTransferProgress,
    },
    RedirectRequest(RdpRedirectRequest),
    CertificateRequest(RdpCertificateRequest),
    Capability {
        session_id: String,
        capability: RdpCapability,
    },
    Error {
        session_id: String,
        error: RdpError,
        fatal: bool,
    },
}

/// Called after an event is enqueued on a session queue, so the consumer can be
/// woken instead of polling for it.
///
/// A callback rather than a channel keeps this crate free of any async runtime;
/// the bounded session queues use a condition variable for producer backpressure
/// and invoke this callback only to schedule a consumer drain.
/// The application supplies a closure that signals its own wake.
///
/// # Contract
///
/// The waker is invoked while the session queue lock is held, so it must not
/// block, must not re-enter the session manager, and must not panic. Signalling
/// an atomic and posting a lightweight GPUI wake event is the intended shape.
pub type QueueWaker = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RdpSessionDrain {
    pub control: Vec<RdpRuntimeEvent>,
    pub frames: Vec<RemoteFrameEvent>,
    pub cursors: Vec<RemoteCursorEvent>,
}

pub const MAX_VNC_CLIPBOARD_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_VNC_INPUT_BATCH: usize = 256;
pub const MAX_VNC_FRAMEBUFFER_WIDTH: u32 = 7680;
pub const MAX_VNC_FRAMEBUFFER_HEIGHT: u32 = 4320;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncSessionConfig {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub relay: Option<crate::relay::RelayEndpoint>,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub password: Option<SecretString>,
    pub security: VncSecurityConfig,
    pub display: VncDisplayConfig,
    pub clipboard: VncClipboardConfig,
    pub reconnect: VncReconnectConfig,
    pub shared: bool,
    pub view_only: bool,
}

impl fmt::Debug for VncSessionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VncSessionConfig")
            .field("name", &self.name)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("security", &self.security)
            .field("display", &self.display)
            .field("clipboard", &self.clipboard)
            .field("reconnect", &self.reconnect)
            .field("shared", &self.shared)
            .field("view_only", &self.view_only)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncSecurityConfig {
    pub mode: VncSecurityMode,
}

impl Default for VncSecurityConfig {
    fn default() -> Self {
        Self {
            mode: VncSecurityMode::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VncSecurityMode {
    #[default]
    Auto,
    None,
    #[serde(alias = "vnc-auth")]
    VncAuth,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncDisplayConfig {
    pub scale_mode: VncScaleMode,
}

impl Default for VncDisplayConfig {
    fn default() -> Self {
        Self {
            scale_mode: VncScaleMode::Fit,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VncScaleMode {
    #[default]
    Fit,
    Stretch,
    Actual,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncClipboardConfig {
    pub enabled: bool,
}

impl Default for VncClipboardConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncReconnectConfig {
    pub enabled: bool,
    pub max_attempts: u32,
}

impl Default for VncReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VncInputEvent {
    Key {
        keysym: u32,
        pressed: bool,
    },
    /// Unicode text committed by an IME or text service. The helper translates
    /// each scalar to its VNC Unicode keysym without synthesizing SAS.
    Text {
        text: String,
    },
    Pointer(RemotePointerEvent),
    ReleaseAllInputs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VncSessionState {
    Connecting,
    Authenticating,
    Negotiating,
    Connected,
    Reconnecting,
    Disconnecting,
    Disconnected,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VncErrorKind {
    Timeout,
    ResourceExhausted,
    Transport,
    Authentication,
    Protocol,
    Encoding,
    Clipboard,
    Internal,
    /// The helper binary was not found beside the application.
    HelperMissing,
    /// The helper exited or panicked while a session was live.
    HelperCrashed,
    /// The helper IPC channel itself failed.
    Ipc,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct VncError {
    pub kind: VncErrorKind,
    pub message: String,
    /// Set only when the helper's effective credentials are missing. Other
    /// authentication failures cannot be resolved by asking for a password.
    /// Older helpers omit this hint and must never trigger an automatic prompt.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub password_required: bool,
}

impl VncError {
    pub fn new(kind: VncErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            password_required: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteDesktopViewState {
    Idle,
    Connecting,
    Connected,
    Reconnecting,
    Disconnecting,
    Disconnected,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteDesktopErrorCategory {
    ResourceExhausted,
    Authentication,
    Certificate,
    Timeout,
    ConnectionRefused,
    Security,
    Transport,
    Session,
    Clipboard,
    Negotiation,
    Encoding,
    HelperMissing,
    HelperCrashed,
    Ipc,
    Protocol,
    Unsupported,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{category:?}: {message}")]
pub struct RemoteDesktopError {
    pub category: RemoteDesktopErrorCategory,
    pub message: String,
}

impl From<RdpError> for RemoteDesktopError {
    fn from(error: RdpError) -> Self {
        let category = match error.kind {
            RdpErrorKind::Authentication => RemoteDesktopErrorCategory::Authentication,
            RdpErrorKind::CertificateRejected => RemoteDesktopErrorCategory::Certificate,
            RdpErrorKind::Timeout => RemoteDesktopErrorCategory::Timeout,
            RdpErrorKind::ConnectionRefused => RemoteDesktopErrorCategory::ConnectionRefused,
            RdpErrorKind::Tls => RemoteDesktopErrorCategory::Security,
            RdpErrorKind::Transport => RemoteDesktopErrorCategory::Transport,
            RdpErrorKind::Session => RemoteDesktopErrorCategory::Session,
            RdpErrorKind::Clipboard => RemoteDesktopErrorCategory::Clipboard,
            RdpErrorKind::Negotiation => RemoteDesktopErrorCategory::Negotiation,
            RdpErrorKind::HelperMissing => RemoteDesktopErrorCategory::HelperMissing,
            RdpErrorKind::HelperCrashed => RemoteDesktopErrorCategory::HelperCrashed,
            RdpErrorKind::Ipc => RemoteDesktopErrorCategory::Ipc,
            RdpErrorKind::Protocol => RemoteDesktopErrorCategory::Protocol,
            RdpErrorKind::ResourceExhausted => RemoteDesktopErrorCategory::ResourceExhausted,
            RdpErrorKind::Unsupported => RemoteDesktopErrorCategory::Unsupported,
        };
        Self {
            category,
            message: error.message,
        }
    }
}

impl From<VncError> for RemoteDesktopError {
    fn from(error: VncError) -> Self {
        let category = match error.kind {
            VncErrorKind::Authentication => RemoteDesktopErrorCategory::Authentication,
            VncErrorKind::Clipboard => RemoteDesktopErrorCategory::Clipboard,
            VncErrorKind::Transport => RemoteDesktopErrorCategory::Transport,
            VncErrorKind::Encoding => RemoteDesktopErrorCategory::Encoding,
            VncErrorKind::Protocol => RemoteDesktopErrorCategory::Protocol,
            VncErrorKind::Internal => RemoteDesktopErrorCategory::Internal,
            VncErrorKind::HelperMissing => RemoteDesktopErrorCategory::HelperMissing,
            VncErrorKind::HelperCrashed => RemoteDesktopErrorCategory::HelperCrashed,
            VncErrorKind::Timeout => RemoteDesktopErrorCategory::Timeout,
            VncErrorKind::ResourceExhausted => RemoteDesktopErrorCategory::ResourceExhausted,
            VncErrorKind::Ipc => RemoteDesktopErrorCategory::Ipc,
        };
        Self {
            category,
            message: error.message,
        }
    }
}

impl From<&RdpSessionState> for RemoteDesktopViewState {
    fn from(state: &RdpSessionState) -> Self {
        match state {
            RdpSessionState::Idle => Self::Idle,
            RdpSessionState::Connecting => Self::Connecting,
            RdpSessionState::Connected => Self::Connected,
            RdpSessionState::Reconnecting => Self::Reconnecting,
            RdpSessionState::Disconnecting => Self::Disconnecting,
            RdpSessionState::Disconnected => Self::Disconnected,
            RdpSessionState::Failed(_) => Self::Failed,
            #[allow(deprecated)]
            RdpSessionState::Active => Self::Connected,
            #[allow(deprecated)]
            RdpSessionState::Closed => Self::Disconnected,
        }
    }
}

impl From<&VncSessionState> for RemoteDesktopViewState {
    fn from(state: &VncSessionState) -> Self {
        match state {
            VncSessionState::Connecting
            | VncSessionState::Authenticating
            | VncSessionState::Negotiating => Self::Connecting,
            VncSessionState::Connected => Self::Connected,
            VncSessionState::Reconnecting => Self::Reconnecting,
            VncSessionState::Disconnecting => Self::Disconnecting,
            VncSessionState::Disconnected => Self::Disconnected,
            VncSessionState::Failed => Self::Failed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncServerKeyRequest {
    pub session_id: String,
    pub generation: u64,
    pub request_id: String,
    pub host: String,
    pub port: u16,
    pub sha256_fingerprint: String,
    pub key_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VncRuntimeEvent {
    ServerKeyRequest(VncServerKeyRequest),
    ServerKeyAuthenticated(VncServerKeyRequest),
    State {
        session_id: String,
        state: VncSessionState,
        message: Option<String>,
    },
    Frame {
        session_id: String,
        event: RemoteFrameEvent,
    },
    Clipboard {
        session_id: String,
        text: String,
    },
    Error {
        session_id: String,
        error: VncError,
        fatal: bool,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VncSessionDrain {
    pub control: Vec<VncRuntimeEvent>,
    pub frames: Vec<RemoteFrameEvent>,
    pub cursors: Vec<RemoteCursorEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RdpControlMessage {
    Ping {
        sequence: u64,
    },
    Pong {
        sequence: u64,
    },
    ClientHello {
        #[serde(default)]
        managed_clipboard: bool,
        version: u32,
        /// Missing on legacy IPC 13 clients, which must not receive new messages.
        #[serde(default)]
        frame_update_complete: bool,
        #[serde(default)]
        server_redirection: bool,
    },
    ServerHello {
        version: u32,
        capabilities: RdpServerCapabilities,
    },
    Connect {
        session_id: String,
        config: RdpSessionConfig,
    },
    DesktopReset {
        session_id: String,
        epoch: u64,
        width: u32,
        height: u32,
    },
    /// All bitmap regions in this submission precede this message on the pipe.
    /// Sequence numbers increase within an epoch and start at one after a reset.
    FrameUpdateComplete {
        session_id: String,
        epoch: u64,
        sequence: u64,
    },
    State {
        session_id: String,
        state: RdpSessionState,
        message: Option<String>,
    },
    Input {
        session_id: String,
        events: Vec<RdpInputEvent>,
    },
    /// Ask the RDP helper to invoke the remote Secure Attention sequence.
    ///
    /// This is deliberately not represented as synthetic Ctrl+Alt+Delete key
    /// events: an RDP server treats SAS as a distinct protocol operation.
    SecureAttention {
        session_id: String,
    },
    Resize {
        session_id: String,
        metrics: RdpDisplayMetrics,
    },
    ManagedClipboard {
        session_id: String,
        command: crate::clipboard::ClipboardCommand,
    },
    ManagedClipboardEvent {
        session_id: String,
        event: crate::clipboard::ClipboardEvent,
    },
    Clipboard {
        session_id: String,
        text: String,
        generation: u64,
    },
    ClipboardTransfer {
        session_id: String,
        progress: RdpClipboardTransferProgress,
    },
    ClipboardTransferCancel {
        session_id: String,
        transfer_id: String,
    },
    RedirectRequest(RdpRedirectRequest),
    CertificateRequest(RdpCertificateRequest),
    RedirectResponse {
        request: RdpRedirectRequest,
        response: RdpRedirectResponse,
    },
    CertificateResponse {
        request_id: String,
        response: RdpCertificateResponse,
    },
    Capability {
        session_id: String,
        capability: RdpCapability,
    },
    Error {
        session_id: String,
        error: RdpError,
        fatal: bool,
    },
    RequestFullFrame {
        session_id: String,
    },
    Disconnect {
        session_id: String,
    },
}

/// Control vocabulary spoken between the application and `nyaterm-vnc-helper`.
///
/// Deliberately narrower than [`RdpControlMessage`]: this VNC path has no dynamic
/// resize, no TLS certificate prompt, and no capability negotiation. Framebuffer
/// and cursor payloads reuse the protocol-neutral binary packets in `crate::ipc`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum VncControlMessage {
    Ping {
        sequence: u64,
    },
    Pong {
        sequence: u64,
    },
    ServerKeyRequest(VncServerKeyRequest),
    ServerKeyAuthenticated(VncServerKeyRequest),
    ServerKeyResponse {
        session_id: String,
        generation: u64,
        request_id: String,
        accept: bool,
    },
    ClientHello {
        version: u32,
    },
    ServerHello {
        version: u32,
        capabilities: VncServerCapabilities,
    },
    Connect {
        session_id: String,
        config: VncSessionConfig,
    },
    /// The helper owns the epoch counter and stamps every frame with it.
    DesktopReset {
        session_id: String,
        epoch: u64,
        width: u32,
        height: u32,
    },
    /// All pixels belonging to this update precede this marker on stdout.
    FrameUpdateComplete {
        session_id: String,
        epoch: u64,
        sequence: u64,
    },
    State {
        session_id: String,
        state: VncSessionState,
        message: Option<String>,
    },
    Input {
        session_id: String,
        events: Vec<VncInputEvent>,
    },
    Clipboard {
        session_id: String,
        text: String,
    },
    Error {
        session_id: String,
        error: VncError,
        fatal: bool,
    },
    RequestFullFrame {
        session_id: String,
    },
    Disconnect {
        session_id: String,
    },
}

pub fn parse_rdp_certificate_policy(value: &str) -> RdpCertificatePolicy {
    match value.trim() {
        "trust_on_first_use" | "trust-on-first-use" | "tofu" => {
            RdpCertificatePolicy::TrustOnFirstUse
        }
        "strict" | "reject_changed" | "reject-changed" => RdpCertificatePolicy::Strict,
        "insecure" | "accept-temporarily" => RdpCertificatePolicy::Insecure,
        _ => RdpCertificatePolicy::Prompt,
    }
}

pub fn parse_rdp_display_mode(value: &str) -> RdpDisplayMode {
    match value.trim() {
        "fixed" | "native" => RdpDisplayMode::Fixed,
        _ => RdpDisplayMode::FitWindow,
    }
}

pub fn parse_rdp_backend(value: &str) -> Result<RdpBackend, RdpError> {
    match value {
        "builtin" | "internal" | "auto" => Ok(RdpBackend::Builtin),
        "windows" | "active-x" => Ok(RdpBackend::Windows),
        _ => Err(RdpError::new(
            RdpErrorKind::Unsupported,
            "Unsupported RDP backend",
        )),
    }
}

impl RdpBackend {
    /// Check before resolving a saved proxy or jump host into a live Relay.
    pub fn validate_route(self, has_relay: bool) -> Result<(), RdpError> {
        if self == Self::Builtin {
            return Ok(());
        }
        if !cfg!(windows) {
            return Err(RdpError::new(
                RdpErrorKind::Unsupported,
                "ActiveX RDP is only available on Windows",
            ));
        }
        if has_relay {
            return Err(RdpError::new(
                RdpErrorKind::Unsupported,
                "ActiveX RDP does not support Relay",
            ));
        }
        Ok(())
    }

    pub fn validate_config(self, config: &RdpSessionConfig) -> Result<(), RdpError> {
        self.validate_route(config.relay.is_some())?;
        if self == Self::Builtin {
            return Ok(());
        }
        if config.windows_authentication.as_deref() != Some("system") {
            return Err(RdpError::new(
                RdpErrorKind::Unsupported,
                "Unsupported Windows system authentication mode for ActiveX RDP",
            ));
        }
        if config.reconnect.max_attempts > 200 {
            return Err(RdpError::new(
                RdpErrorKind::Unsupported,
                "ActiveX reconnect attempts must be between 0 and 200",
            ));
        }
        Ok(())
    }
}

pub fn parse_rdp_clipboard_mode(value: &str) -> RdpClipboardMode {
    match value.trim() {
        "disabled" | "off" => RdpClipboardMode::Disabled,
        "text-and-files" => RdpClipboardMode::TextAndFiles,
        _ => RdpClipboardMode::TextOnly,
    }
}

pub fn parse_vnc_security_mode(value: &str) -> VncSecurityMode {
    match value.trim() {
        "none" => VncSecurityMode::None,
        "vnc-auth" | "vnc_auth" | "password" => VncSecurityMode::VncAuth,
        _ => VncSecurityMode::Auto,
    }
}

pub fn parse_vnc_scale_mode(value: &str) -> VncScaleMode {
    match value.trim() {
        "stretch" => VncScaleMode::Stretch,
        "actual" | "actual-size" => VncScaleMode::Actual,
        _ => VncScaleMode::Fit,
    }
}

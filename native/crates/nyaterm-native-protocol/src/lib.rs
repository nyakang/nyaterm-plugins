//! Bounded, versioned contracts shared by the application and native helpers.
pub mod protocol;
pub use protocol::{
    CommittedTextError, CursorPosition, CursorShape, CursorVisibility, MAX_COMMITTED_TEXT_BYTES,
    MAX_HEIGHT, MAX_VNC_CLIPBOARD_TEXT_BYTES, MAX_VNC_FRAMEBUFFER_HEIGHT,
    MAX_VNC_FRAMEBUFFER_WIDTH, MAX_VNC_INPUT_BATCH, MAX_WIDTH, MIN_HEIGHT, MIN_WIDTH,
    PROTOCOL_VERSION, PixelFormat, QueueWaker, RDP_ALL_DRIVES, RdpAudioMode, RdpBackend,
    RdpCapability, RdpCertificatePolicy, RdpCertificateRequest, RdpCertificateResponse,
    RdpClipboardConfig, RdpClipboardMode, RdpClipboardTransferProgress, RdpClipboardTransferStatus,
    RdpControlMessage, RdpDisplayConfig, RdpDisplayMetrics, RdpDisplayMode, RdpError, RdpErrorKind,
    RdpFrameEvent, RdpInputEvent, RdpPointerButton, RdpReconnectConfig, RdpRedirectionConfig,
    RdpRuntimeEvent, RdpServerCapabilities, RdpSessionConfig, RdpSessionDrain, RdpSessionState,
    RemoteCursorEvent, RemoteDesktopError, RemoteDesktopErrorCategory, RemoteDesktopViewState,
    RemoteFrameEvent, RemotePoint, RemotePointerButton, RemotePointerEvent, RemoteWheelAxis,
    VNC_PROTOCOL_VERSION, VncClipboardConfig, VncControlMessage, VncDisplayConfig, VncError,
    VncErrorKind, VncInputEvent, VncReconnectConfig, VncRuntimeEvent, VncScaleMode,
    VncSecurityConfig, VncSecurityMode, VncServerCapabilities, VncServerKeyRequest,
    VncSessionConfig, VncSessionDrain, VncSessionState, default_rdp_drive_letters,
    parse_rdp_audio_mode, parse_rdp_backend, parse_rdp_certificate_policy,
    parse_rdp_clipboard_mode, parse_rdp_display_mode, parse_rdp_drive_letters,
    parse_vnc_scale_mode, parse_vnc_security_mode, toggle_rdp_drive_letter,
    validate_committed_text,
};

pub mod frame;
pub use frame::{
    DirtyRect, Framebuffer, FramebufferError, FramebufferLimits, RDP_FRAMEBUFFER_LIMITS,
    VNC_FRAMEBUFFER_LIMITS, frame_batch_len, merge_dirty_rects, validate_framebuffer_dimensions,
};

pub mod pixel_memory;
pub use pixel_memory::{PIXEL_MEMORY_LIMIT, PixelBudget, PixelBuffer, PixelLease};

pub mod ipc;
pub use ipc::{
    CONTROL_PAYLOAD_LIMIT, CURSOR_PAYLOAD_LIMIT, FRAME_PAYLOAD_LIMIT, HEADER_LEN, Packet,
    PacketReader, PacketType, decode_control, decode_cursor_packet, decode_cursor_packet_owned,
    decode_frame_packet, decode_frame_packet_owned, decode_vnc_control, encode_control,
    encode_cursor_packet, encode_frame_packet, encode_frame_packet_owned, encode_vnc_control,
    read_packet, read_packet_budgeted, read_packet_inner, write_packet, write_packet_into,
};

pub mod android;
pub mod relay;
pub mod secret;
pub mod spice;

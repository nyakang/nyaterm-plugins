//! Opt-in clipboard ownership. Payloads deliberately redact Debug output.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardAccess {
    Active,
    Drain,
    Disabled,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardContent {
    Empty,
    Text(String),
    Files(Vec<PathBuf>),
}

impl std::fmt::Debug for ClipboardContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "Empty",
            Self::Text(_) => "Text([redacted])",
            Self::Files(_) => "Files([redacted])",
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum ClipboardCommand {
    Authorize {
        epoch: u64,
        access: ClipboardAccess,
        cache_root: PathBuf,
    },
    Snapshot {
        epoch: u64,
        revision: u64,
        content: ClipboardContent,
    },
}
impl std::fmt::Debug for ClipboardCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClipboardCommand([redacted])")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardEvent {
    Applied {
        epoch: u64,
        revision: u64,
        accepted: bool,
    },
    Started {
        epoch: u64,
        request: u64,
    },
    Ready {
        epoch: u64,
        request: u64,
        content: ClipboardContent,
        transfer_id: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use crate::clipboard::{ClipboardAccess, ClipboardCommand, ClipboardContent, ClipboardEvent};
    use crate::ipc::{decode_control, encode_control};
    use crate::protocol::{RdpControlMessage, RdpServerCapabilities};

    #[test]
    fn legacy_handshake_does_not_opt_in_to_managed_clipboard() {
        let hello: RdpControlMessage =
            serde_json::from_str(r#"{"ClientHello":{"version":13}}"#).unwrap();
        assert!(matches!(
            hello,
            RdpControlMessage::ClientHello {
                managed_clipboard: false,
                ..
            }
        ));
        let capabilities: RdpServerCapabilities =
            serde_json::from_str(r#"{"committed_unicode_text":true,"secure_attention":true}"#)
                .unwrap();
        assert!(!capabilities.managed_clipboard);
    }

    #[test]
    fn managed_clipboard_messages_round_trip_without_disclosing_payloads_in_debug() {
        let command = ClipboardCommand::Authorize {
            epoch: 9,
            access: ClipboardAccess::Active,
            cache_root: "/private/cache".into(),
        };
        assert!(!format!("{command:?}").contains("/private"));
        let message = RdpControlMessage::ManagedClipboard {
            session_id: "session".into(),
            command,
        };
        assert!(matches!(
            decode_control(&encode_control(&message).unwrap()).unwrap(),
            RdpControlMessage::ManagedClipboard {
                command: ClipboardCommand::Authorize { epoch: 9, .. },
                ..
            }
        ));
        let event = ClipboardEvent::Ready {
            epoch: 9,
            request: 2,
            content: ClipboardContent::Text("private fixture".into()),
            transfer_id: None,
        };
        assert!(!format!("{event:?}").contains("private fixture"));
        let message = RdpControlMessage::ManagedClipboardEvent {
            session_id: "session".into(),
            event: event.clone(),
        };
        let RdpControlMessage::ManagedClipboardEvent { event: decoded, .. } =
            decode_control(&encode_control(&message).unwrap()).unwrap()
        else {
            panic!("clipboard event");
        };
        assert_eq!(event, decoded);
    }

    #[test]
    fn oversized_clipboard_payloads_fail_before_entering_the_pipe() {
        let message = RdpControlMessage::ManagedClipboard {
            session_id: "session".into(),
            command: ClipboardCommand::Snapshot {
                epoch: 1,
                revision: 1,
                content: ClipboardContent::Text("x".repeat(crate::ipc::CONTROL_PAYLOAD_LIMIT)),
            },
        };
        assert!(encode_control(&message).is_err());
    }
}

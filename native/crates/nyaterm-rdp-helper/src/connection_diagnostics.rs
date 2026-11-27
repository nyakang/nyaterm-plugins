//! Opt-in connection milestones, without peer identities or protocol payloads.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::sync::Mutex;
use std::time::Instant;

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

const MAX_LOG_BYTES: u64 = 1024 * 1024;

pub(crate) fn install() {
    let Some(path) = std::env::var_os("NYATERM_RDP_PHASE_LOG") else {
        return;
    };
    let Ok(file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let subscriber = ConnectionDiagnostics {
        file: Mutex::new(file),
        started: Instant::now(),
    };
    subscriber.write("helper_started");
    let _ = tracing::subscriber::set_global_default(subscriber);
}

struct ConnectionDiagnostics {
    file: Mutex<File>,
    started: Instant,
}

impl ConnectionDiagnostics {
    fn write(&self, phase: &'static str) {
        if let Ok(mut file) = self.file.lock() {
            let line = format!(
                "pid={} elapsed_ms={} phase={phase}\n",
                std::process::id(),
                self.started.elapsed().as_millis()
            );
            // The opt-in file persists across retries; never let frame milestones
            // grow it indefinitely or truncate diagnostics from an earlier attempt.
            if file.metadata().is_ok_and(|metadata| {
                metadata.len().saturating_add(line.len() as u64) <= MAX_LOG_BYTES
            }) {
                let _ = file.write_all(line.as_bytes());
            }
        }
    }
}

impl Subscriber for ConnectionDiagnostics {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.is_event()
            && matches!(
                metadata.target(),
                "ironrdp_async::framed"
                    | "ironrdp_async::connector"
                    | "ironrdp_client::rdp"
                    | "ironrdp_connector::connection"
                    | "ironrdp_dvc::client"
                    | "ironrdp_egfx::client"
                    | "ironrdp_client::h264"
                    | "nyaterm_rdp_phase"
            )
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut milestone = Milestone::default();
        event.record(&mut milestone);
        if let Some(phase) = milestone.state.or(milestone.message) {
            self.write(phase);
        }
    }
}

#[derive(Default)]
struct Milestone {
    state: Option<&'static str>,
    message: Option<&'static str>,
}

impl Visit for Milestone {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "connector.state" {
            self.state = match value {
                "ConnectionInitiationWaitConfirm" => Some("waiting_x224_confirm"),
                "Credssp" => Some("credssp"),
                "RdstlsWaitCapabilities" => Some("waiting_rdstls_capabilities"),
                "RdstlsWaitResult" => Some("waiting_rdstls_result"),
                "BasicSettingsExchangeWaitResponse" => Some("waiting_mcs_response"),
                "ChannelConnection" => Some("channel_connection"),
                "ConnectTimeAutoDetection" => Some("connection_autodetection"),
                "LicensingExchange" => Some("licensing"),
                "CapabilitiesExchange" => Some("capabilities_exchange"),
                "ConnectionFinalization" => Some("connection_finalization"),
                _ => None,
            };
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        // Only an allowlisted static label reaches the file. Never format other
        // fields: they may contain credentials, hostnames or packet contents.
        if field.name() == "message" {
            self.message = match format!("{value:?}").as_str() {
                "Connection Initiation" => Some("x224_request"),
                "TLS upgrade" => Some("tls_handshake"),
                "Begin NLA using CredSSP" => Some("credssp"),
                "Connected with success" => Some("connection_activated"),
                "Connected to the redirection target" => Some("redirect_target_tcp_connected"),
                "Server redirected the connection" => Some("server_redirected"),
                "Redirect approved" => Some("redirect_approved"),
                "Waiting for first frame" => Some("waiting_first_frame"),
                "First frame received" => Some("first_frame_received"),
                "Connection timeout" => Some("connection_timeout"),
                "Advertising EGFX capabilities" => Some("gfx_capabilities_advertised"),
                "Graphics reset" => Some("gfx_reset"),
                "Surface created" => Some("gfx_surface_created"),
                "Surface mapped to output" => Some("gfx_surface_mapped"),
                "StartFrame" => Some("gfx_start_frame"),
                "Sending FrameAcknowledge" => Some("gfx_frame_ack"),
                "Unable to create the H.264 decoder" => Some("h264_decoder_unavailable"),
                "The H.264 decoder did not start in time" => Some("h264_decoder_start_timeout"),
                "The H.264 decoder is unavailable; advertising no AVC capability set" => {
                    Some("gfx_no_avc")
                }
                "No H.264 decoder configured, skipping AVC420 frame" => {
                    Some("gfx_avc_frame_skipped")
                }
                message if message.starts_with("Got DVC Capabilities Request PDU:") => {
                    Some("dvc_capabilities_received")
                }
                message if message.starts_with("Send DVC Capabilities Response PDU:") => {
                    Some("dvc_capabilities_response_generated")
                }
                message if message.starts_with("Got DVC Create Request PDU:") => {
                    Some("dvc_create_received")
                }
                message if message.starts_with("Send DVC Create Response PDU:") => {
                    Some("dvc_create_response")
                }
                _ => None,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{ConnectionDiagnostics, MAX_LOG_BYTES};

    #[test]
    fn full_diagnostic_log_is_preserved_without_growing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("phases.log");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_LOG_BYTES).unwrap();
        let subscriber = ConnectionDiagnostics {
            file: std::sync::Mutex::new(file),
            started: std::time::Instant::now(),
        };
        subscriber.write("first_frame_received");
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_LOG_BYTES);
        assert!(fs::read(path).unwrap().iter().all(|byte| *byte == 0));
    }

    #[test]
    fn milestone_log_drops_credentials_payloads_and_unrecognized_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("phases.log");
        let subscriber = ConnectionDiagnostics {
            file: std::sync::Mutex::new(fs::File::create(&path).unwrap()),
            started: std::time::Instant::now(),
        };
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "ironrdp_async::framed", { connector.state = "RdstlsWaitResult", password = "secret-password", payload = ?[13, 17, 19] }, "Wait for PDU");
            tracing::debug!(target: "ironrdp_async::framed", { connector.state = "secret-identity" }, "secret-message");
            tracing::debug!(target: "ironrdp_client::rdp", host = "secret-host", "TLS upgrade");
        });
        let log = fs::read_to_string(path).unwrap();
        assert!(log.contains("phase=waiting_rdstls_result"));
        assert!(log.contains("phase=tls_handshake"));
        assert_eq!(log.lines().count(), 2);
        assert!(!log.contains("secret"));
        assert!(!log.contains("13, 17, 19"));
    }
}

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use nyaterm_remote_desktop::spice::ipc::{decode_control, encode_control};
use nyaterm_remote_desktop::spice::protocol::{
    SPICE_PROTOCOL_VERSION, SpiceControl, SpiceCredentialLifetime, SpiceErrorKind,
    SpiceSessionConfig, SpiceSessionState,
};
use nyaterm_remote_desktop::spice::settings::{SpiceEndpoint, SpiceSettings};
use nyaterm_remote_desktop::{read_packet, write_packet};

struct Helper {
    child: Child,
    stdin: ChildStdin,
    events: Receiver<SpiceControl>,
}

impl Helper {
    fn spawn(native: bool) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nyaterm-spice-helper"));
        if !native {
            command.env(
                "NYATERM_SPICE_RUNTIME",
                std::env::temp_dir().join("nyaterm-missing-spice-runtime"),
            );
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (sender, events) = mpsc::sync_channel(32);
        std::thread::spawn(move || {
            while let Ok(Some(packet)) = read_packet(&mut stdout) {
                let Ok(message) = decode_control(&packet) else {
                    break;
                };
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            events,
        }
    }

    fn send(&mut self, control: SpiceControl) {
        write_packet(&mut self.stdin, &encode_control(&control).unwrap()).unwrap();
    }

    fn recv(&self) -> SpiceControl {
        self.events
            .recv_timeout(Duration::from_secs(5))
            .expect("helper response deadline")
    }

    fn hello(&mut self) {
        self.send(SpiceControl::Hello {
            version: SPICE_PROTOCOL_VERSION,
        });
        assert_eq!(
            self.recv(),
            SpiceControl::HelloAck {
                version: SPICE_PROTOCOL_VERSION
            }
        );
    }

    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "helper did not exit within deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn config(port: u16) -> SpiceSessionConfig {
    let mut settings = SpiceSettings::default();
    settings.security.require_tls = false;
    settings.audio_playback = false;
    SpiceSessionConfig {
        session_id: "lifecycle".into(),
        endpoint: SpiceEndpoint {
            extra: Default::default(),
            host: "127.0.0.1".into(),
            port: Some(port),
            tls_port: None,
            proxy: None,
        },
        username: "".into(),
        password: "fixture-ticket".into(),
        settings,
        credential_lifetime: SpiceCredentialLifetime::Temporary,
        tcp_relay: None,
        tls_relay: None,
    }
}

#[test]
fn handshake_ping_and_normal_disconnect_do_not_require_native_libraries() {
    let mut helper = Helper::spawn(false);
    helper.hello();
    helper.send(SpiceControl::Ping { sequence: 42 });
    assert_eq!(helper.recv(), SpiceControl::Pong { sequence: 42 });
    helper.send(SpiceControl::Disconnect);
    assert_eq!(
        helper.recv(),
        SpiceControl::State {
            state: SpiceSessionState::Disconnected
        }
    );
    assert!(helper.wait().success());
}

#[test]
fn missing_runtime_is_a_typed_fatal_error_after_connecting_state() {
    let mut helper = Helper::spawn(false);
    helper.hello();
    helper.send(SpiceControl::Connect {
        config: Box::new(config(5900)),
    });
    assert_eq!(
        helper.recv(),
        SpiceControl::State {
            state: SpiceSessionState::Connecting
        }
    );
    assert_eq!(
        helper.recv(),
        SpiceControl::Error {
            error: SpiceErrorKind::RuntimeMissing,
            fatal: true
        }
    );
    assert!(!helper.wait().success());
}

#[test]
fn wrong_version_and_server_messages_are_rejected() {
    for command in [
        SpiceControl::Hello {
            version: SPICE_PROTOCOL_VERSION + 1,
        },
        SpiceControl::HelloAck {
            version: SPICE_PROTOCOL_VERSION,
        },
        SpiceControl::Pong { sequence: 1 },
    ] {
        let mut helper = Helper::spawn(false);
        helper.send(command);
        assert_eq!(
            helper.recv(),
            SpiceControl::Error {
                error: SpiceErrorKind::Protocol,
                fatal: true
            }
        );
        assert!(!helper.wait().success());
    }
    let mut helper = Helper::spawn(false);
    helper.hello();
    helper.send(SpiceControl::Capabilities {
        capabilities: Default::default(),
    });
    assert_eq!(
        helper.recv(),
        SpiceControl::Error {
            error: SpiceErrorKind::Protocol,
            fatal: true
        }
    );
    assert!(!helper.wait().success());
}

#[test]
fn oversized_or_wrong_direction_packets_fail_without_waiting_for_body() {
    for header in [[1, 255, 255, 255, 127], [2, 1, 0, 0, 0], [255, 0, 0, 0, 0]] {
        let mut helper = Helper::spawn(false);
        helper.stdin.write_all(&header).unwrap();
        helper.stdin.flush().unwrap();
        assert_eq!(
            helper.recv(),
            SpiceControl::Error {
                error: SpiceErrorKind::Protocol,
                fatal: true
            }
        );
        assert!(!helper.wait().success());
    }
}

#[test]
#[ignore = "requires a prepared native SPICE runtime"]
fn native_connect_stall_still_accepts_ping_and_cancellation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut helper = Helper::spawn(true);
    helper.hello();
    helper.send(SpiceControl::Connect {
        config: Box::new(config(listener.local_addr().unwrap().port())),
    });
    assert_eq!(
        helper.recv(),
        SpiceControl::State {
            state: SpiceSessionState::Connecting
        }
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "native handshake did not arrive");
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("fixture listener: {error}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut magic = [0; 4];
    stream.read_exact(&mut magic).unwrap();
    assert_eq!(&magic, b"REDQ");
    helper.send(SpiceControl::Ping { sequence: 99 });
    assert_eq!(helper.recv(), SpiceControl::Pong { sequence: 99 });
    helper.send(SpiceControl::Disconnect);
    assert_eq!(
        helper.recv(),
        SpiceControl::State {
            state: SpiceSessionState::Disconnected
        }
    );
    assert!(helper.wait().success());
}

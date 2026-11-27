use nyaterm_native_protocol::x11::{self, Command as Control, Event, Failure};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn incompatible_handshake_reports_failure_and_exits() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nyaterm-x11-helper"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    x11::write(
        child.stdin.as_mut().unwrap(),
        &Control::Start {
            version: u32::MAX,
            clipboard: false,
        },
    )
    .unwrap();
    assert!(matches!(
        x11::read::<Event>(child.stdout.as_mut().unwrap()).unwrap(),
        Event::Failed {
            kind: Failure::IncompatibleProtocol
        }
    ));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("helper did not exit");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn stop_before_start_exits_without_launching_a_server() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nyaterm-x11-helper"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    x11::write(child.stdin.as_mut().unwrap(), &Control::Stop).unwrap();
    assert!(child.wait().unwrap().success());
}

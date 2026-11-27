use crate::process::OwnedChild;
use nyaterm_native_protocol::secret::SecretString;
use nyaterm_native_protocol::x11::{self, Command, Event, Failure, X11_PROTOCOL_VERSION};
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

fn authority(file: &mut File, display: u16, cookie: &[u8]) -> io::Result<()> {
    file.write_all(&65535u16.to_be_bytes())?; // FamilyWild, private file scoped to this instance.
    for field in [
        &[][..],
        display.to_string().as_bytes(),
        b"MIT-MAGIC-COOKIE-1",
        cookie,
    ] {
        file.write_all(&(field.len() as u16).to_be_bytes())?;
        file.write_all(field)?;
    }
    file.sync_all()
}

fn accepts(display: u16, cookie: &[u8]) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(
        &([127, 0, 0, 1], 6000 + display).into(),
        Duration::from_millis(100),
    ) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let mut packet = Zeroizing::new(vec![b'l', 0, 11, 0, 0, 0, 18, 0, 16, 0, 0, 0]);
    packet.extend_from_slice(b"MIT-MAGIC-COOKIE-1\0\0");
    packet.extend_from_slice(cookie);
    let mut header = [0; 8];
    stream.write_all(&packet).is_ok() && stream.read_exact(&mut header).is_ok() && header[0] == 1
}

pub(super) fn run(clipboard: bool) -> io::Result<()> {
    let stopped = Arc::new(AtomicBool::new(false));
    let signal = stopped.clone();
    // EOF also stops the server after a parent crash. The parent's Job Object
    // covers a stuck reader/helper, including descendants with inherited pipes.
    std::thread::spawn(move || {
        let _ = x11::read::<Command>(&mut std::io::stdin().lock());
        signal.store(true, Ordering::Release);
    });
    let root = std::env::current_exe()?
        .parent()
        .ok_or_else(|| io::Error::other("Missing component root"))?
        .join("vcxsrv");
    let private = tempfile::Builder::new().prefix("nyaterm-x11-").tempdir()?;
    let mut cookie = Zeroizing::new([0u8; 16]);
    getrandom::fill(cookie.as_mut()).map_err(|_| io::Error::other("Random source unavailable"))?;
    let start = u16::from_le_bytes([cookie[0], cookie[1]]) % 50000 + 100;
    let mut active = None;
    for offset in 0..32 {
        if stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        let display = start + offset;
        // A held per-port file lock coordinates all NyaTerm helper instances;
        // the server's bind remains authoritative against external processes.
        let lock_path = std::env::temp_dir().join(format!("nyaterm-x11-display-{display}.lock"));
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        if lock.try_lock().is_err() {
            continue;
        }
        if TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 6000 + display)).is_err() {
            continue;
        }
        let auth_path = private.path().join(format!("authority-{display}"));
        let mut auth = crate::private_file::create_private_file(&auth_path)?;
        authority(&mut auth, display, cookie.as_ref())?;
        drop(auth);
        let mut command = ProcessCommand::new(root.join("vcxsrv.exe"));
        command
            .current_dir(&root)
            .arg(format!(":{display}"))
            .args([
                "-multiwindow",
                "-noreset",
                "-listen",
                "tcp",
                "-nolisten",
                "inet6",
                "-auth",
            ])
            .arg(auth_path)
            .args([
                if clipboard {
                    "-clipboard"
                } else {
                    "-noclipboard"
                },
                "-logfile",
                "NUL",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Do not load the user's Xauthority, X0.hosts or XWin configuration.
        command.env("HOME", private.path()).env(
            "XAUTHORITY",
            private.path().join(format!("authority-{display}")),
        );
        let mut child = OwnedChild::spawn(&mut command)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && !stopped.load(Ordering::Acquire) {
            if child.child_mut().try_wait()?.is_some() {
                break;
            }
            if accepts(display, cookie.as_ref()) {
                let mut wrong = Zeroizing::new(*cookie);
                wrong[0] ^= 255;
                if accepts(display, wrong.as_ref()) {
                    return Err(io::Error::other("X11 authentication enforcement failed"));
                }
                active = Some((child, lock, display));
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if active.is_some() {
            break;
        }
    }
    let Some((mut child, _lock, display)) = active else {
        return Err(io::Error::other("X11 startup failed"));
    };
    let hex = SecretString::new(
        cookie
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    );
    x11::write(
        &mut std::io::stdout().lock(),
        &Event::Ready {
            version: X11_PROTOCOL_VERSION,
            display,
            cookie: hex,
        },
    )?;
    while !stopped.load(Ordering::Acquire) {
        if child.child_mut().try_wait()?.is_some() {
            x11::write(
                &mut std::io::stdout().lock(),
                &Event::Failed {
                    kind: Failure::ServerExited,
                },
            )?;
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    child.terminate();
    x11::write(&mut std::io::stdout().lock(), &Event::Stopped)
}

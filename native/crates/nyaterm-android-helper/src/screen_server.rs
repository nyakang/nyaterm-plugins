//! Component-owned scrcpy resources and device-side screen process lifetime.
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use nyaterm_native_protocol::android::settings::{AndroidScreenSettings, DeviceBinding};

const VERSION: &str = "5.0.1";
const SERVER: &[u8] = include_bytes!("../assets/scrcpy-server-v5.0.1");

pub struct ScreenServer {
    adb: PathBuf,
    binding: DeviceBinding,
    scid: String,
    remote: String,
    port: u16,
    child: Option<Child>,
}

fn validate_identity(scid: &str, binding: &DeviceBinding) -> io::Result<()> {
    if scid.len() != 8
        || !scid.bytes().all(|b| b.is_ascii_hexdigit())
        || binding.server.port() == 0
        || binding.serial.is_empty()
        || binding.serial.len() > 4096
        || binding.serial.chars().any(char::is_control)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid screen device identity",
        ));
    }
    Ok(())
}

impl ScreenServer {
    pub fn start(
        binding: DeviceBinding,
        scid: String,
        settings: AndroidScreenSettings,
    ) -> io::Result<Self> {
        validate_identity(&scid, &binding)?;
        settings.validate().map_err(io::Error::other)?;
        let executable = std::env::current_exe()?;
        let root = executable
            .parent()
            .ok_or_else(|| io::Error::other("component root"))?;
        let adb = root
            .join("adb")
            .join(if cfg!(windows) { "adb.exe" } else { "adb" });
        if !adb.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "component adb resource missing",
            ));
        }
        let mut server = Self {
            adb,
            binding,
            remote: format!("/data/local/tmp/nyaterm-scrcpy-{scid}.jar"),
            scid,
            port: 0,
            child: None,
        };
        let serial = server.run(&["get-serialno"])?;
        if serial.trim() != server.binding.serial {
            return Err(io::Error::other("device binding changed"));
        }
        let mut resource = tempfile::NamedTempFile::new()?;
        resource.write_all(SERVER)?;
        resource.flush()?;
        let resource_path = resource
            .path()
            .to_str()
            .ok_or_else(|| io::Error::other("server resource path"))?;
        server.run(&["push", resource_path, &server.remote])?;
        let destination = format!("localabstract:scrcpy_{}", server.scid);
        server.port = server
            .run(&["forward", "tcp:0", &destination])?
            .trim()
            .parse()
            .map_err(|_| io::Error::other("screen forward port"))?;
        if server.port == 0 {
            return Err(io::Error::other("screen forward port"));
        }
        // Only validated numeric values and a fixed, owned path enter this shell command.
        let command = format!(
            "CLASSPATH={} exec app_process / com.genymobile.scrcpy.Server {VERSION} scid={} tunnel_forward=true audio=false control=true video_codec=h264 max_size={} max_fps={} video_bit_rate={} clipboard_autosync=false cleanup=false power_on={} capture_orientation={} log_level=error",
            server.remote,
            server.scid,
            settings.max_size,
            settings.max_fps,
            settings.bit_rate,
            settings.power_on,
            settings.capture_orientation.server_value()
        );
        server.child = Some(
            server
                .command()
                .args(["shell", &command])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        );
        Ok(server)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.adb);
        command.args([
            "-H",
            &self.binding.server.ip().to_string(),
            "-P",
            &self.binding.server.port().to_string(),
        ]);
        if let Some(id) = self.binding.transport_id {
            command.args(["-t", &id.to_string()]);
        } else {
            command.args(["-s", &self.binding.serial]);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command
    }

    fn run(&self, args: &[&str]) -> io::Result<String> {
        // File-backed output avoids blocking on full pipes while applying a deadline.
        let output = tempfile::tempfile()?;
        let mut child = self
            .command()
            .args(args)
            .stdin(Stdio::null())
            .stdout(output.try_clone()?)
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait()? {
                if !status.success() {
                    return Err(io::Error::other("screen device operation failed"));
                }
                break;
            }
            if Instant::now() >= deadline || output.metadata()?.len() > 64 * 1024 {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "screen device operation exceeded its limit",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        use std::io::{Seek, SeekFrom};
        let mut output = output;
        output.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        output.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 64 * 1024 {
            return Err(io::Error::other("screen device response limit"));
        }
        String::from_utf8(bytes).map_err(|_| io::Error::other("screen device response encoding"))
    }

    pub fn endpoint(&self) -> SocketAddr {
        SocketAddr::new(self.binding.server.ip(), self.port)
    }
}

impl Drop for ScreenServer {
    fn drop(&mut self) {
        let command = format!(
            "for p in /proc/[0-9]*/cmdline; do [ -r \"$p\" ] || continue; tr '\\000' '\\n' < \"$p\" | grep -Fx -- 'com.genymobile.scrcpy.Server' >/dev/null || continue; tr '\\000' '\\n' < \"$p\" | grep -Fx -- 'scid={}' >/dev/null || continue; p=${{p%/cmdline}}; p=${{p##*/}}; kill \"$p\"; done; rm -f -- {}",
            self.scid, self.remote
        );
        let _ = self.run(&["shell", &command]);
        // Match the unique destination before removing a forward; a restarted ADB
        // daemon may have reused its TCP port for a different screen session.
        if let Ok(list) = self.run(&["forward", "--list"]) {
            let destination = format!("localabstract:scrcpy_{}", self.scid);
            for line in list.lines() {
                let parts: Vec<_> = line.split_whitespace().collect();
                if parts.len() == 3 && parts[0] == self.binding.serial && parts[2] == destination {
                    let _ = self.run(&["forward", "--remove", parts[1]]);
                }
            }
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::screen_server::validate_identity;
    use nyaterm_native_protocol::android::settings::DeviceBinding;

    #[test]
    fn screen_identity_rejects_shell_metacharacters_before_starting_adb() {
        let binding = DeviceBinding {
            server: "127.0.0.1:5037".parse().unwrap(),
            serial: "fixture".into(),
            transport_id: Some(7),
            generation: 1,
        };
        assert!(validate_identity("1234abcd", &binding).is_ok());
        for value in ["", "../12345", "a;echo x", "123456789"] {
            assert!(validate_identity(value, &binding).is_err());
        }
    }
}

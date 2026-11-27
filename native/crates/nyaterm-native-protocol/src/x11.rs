//! Private, bounded control pipe for the X11 process supervisor. No X11 data travels here.
use crate::secret::SecretString;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};
use zeroize::Zeroizing;

pub const X11_PROTOCOL_VERSION: u32 = 1;
pub const MAX_CONTROL_BYTES: usize = 4096;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Command {
    Start { version: u32, clipboard: bool },
    Stop,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Event {
    Ready {
        version: u32,
        display: u16,
        cookie: SecretString,
    },
    Failed {
        kind: Failure,
    },
    Stopped,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Failure {
    UnsupportedPlatform,
    IncompatibleProtocol,
    Startup,
    ServerExited,
}

pub fn read<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix)?;
    let length = u32::from_le_bytes(prefix) as usize;
    if length == 0 || length > MAX_CONTROL_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid X11 control size",
        ));
    }
    let mut bytes = Zeroizing::new(vec![0; length]);
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid X11 control message"))
}

pub fn write<T: Serialize>(writer: &mut impl Write, value: &T) -> io::Result<()> {
    let bytes = Zeroizing::new(
        serde_json::to_vec(value).map_err(|_| io::Error::other("Invalid X11 control message"))?,
    );
    if bytes.len() > MAX_CONTROL_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "X11 control size exceeded",
        ));
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use crate::secret::SecretString;
    use crate::x11::{Command, Event, MAX_CONTROL_BYTES, X11_PROTOCOL_VERSION, read, write};
    #[test]
    fn control_roundtrip_and_secret_redaction() {
        let event = Event::Ready {
            version: X11_PROTOCOL_VERSION,
            display: 17,
            cookie: SecretString::new("private-cookie"),
        };
        assert!(!format!("{event:?}").contains("private-cookie"));
        let mut bytes = Vec::new();
        write(&mut bytes, &event).unwrap();
        assert!(matches!(
            read::<Event>(&mut bytes.as_slice()).unwrap(),
            Event::Ready { display: 17, .. }
        ));
    }
    #[test]
    fn oversized_frames_are_rejected_before_allocation() {
        let prefix = (MAX_CONTROL_BYTES as u32 + 1).to_le_bytes();
        assert!(read::<Command>(&mut prefix.as_slice()).is_err());
    }
}

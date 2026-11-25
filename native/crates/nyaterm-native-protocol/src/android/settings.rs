use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AndroidScreenSettings {
    pub max_size: u32,
    pub max_fps: u32,
    pub bit_rate: u32,
    pub power_on: bool,
    pub capture_orientation: CaptureOrientation,
}

impl Default for AndroidScreenSettings {
    fn default() -> Self {
        Self {
            max_size: 1280,
            max_fps: 30,
            bit_rate: 8_000_000,
            power_on: false,
            capture_orientation: CaptureOrientation::FollowDevice,
        }
    }
}

/// Capture transforms are applied by the server, before input geometry is reported.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureOrientation {
    #[default]
    FollowDevice,
    LockInitial,
    Degrees0,
    Degrees90,
    Degrees180,
    Degrees270,
}
impl CaptureOrientation {
    pub const ALL: [Self; 6] = [
        Self::FollowDevice,
        Self::LockInitial,
        Self::Degrees0,
        Self::Degrees90,
        Self::Degrees180,
        Self::Degrees270,
    ];
    pub const fn server_value(self) -> &'static str {
        match self {
            Self::FollowDevice => "0",
            Self::LockInitial => "@",
            Self::Degrees0 => "@0",
            Self::Degrees90 => "@90",
            Self::Degrees180 => "@180",
            Self::Degrees270 => "@270",
        }
    }
    pub const fn label_key(self) -> &'static str {
        match self {
            Self::FollowDevice => "android.orientationFollow",
            Self::LockInitial => "android.orientationInitial",
            Self::Degrees0 => "android.orientation0",
            Self::Degrees90 => "android.orientation90",
            Self::Degrees180 => "android.orientation180",
            Self::Degrees270 => "android.orientation270",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AndroidScreenValidationError {
    #[error("Android screen maximum edge must be between 128 and 2560")]
    MaxSize,
    #[error("Android screen maximum FPS must be between 1 and 120")]
    MaxFps,
    #[error("Android screen bit rate must be between 100000 and 100000000")]
    BitRate,
}

impl AndroidScreenSettings {
    pub fn validate(self) -> Result<(), AndroidScreenValidationError> {
        if !(128..=2560).contains(&self.max_size) {
            return Err(AndroidScreenValidationError::MaxSize);
        }
        if !(1..=120).contains(&self.max_fps) {
            return Err(AndroidScreenValidationError::MaxFps);
        }
        if !(100_000..=100_000_000).contains(&self.bit_rate) {
            return Err(AndroidScreenValidationError::BitRate);
        }
        Ok(())
    }
}

/// Immutable binding. Reconnection creates a new binding and generation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceBinding {
    pub server: SocketAddr,
    pub serial: String,
    pub transport_id: Option<u64>,
    pub generation: u64,
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StayOnMode {
    Always,      // svc power stayon true / settings put global stay_on_while_plugged_in 7
    UsbOnly,     // svc power stayon usb / settings put global stay_on_while_plugged_in 2
    SystemDefault, // svc power stayon false / settings put global stay_on_while_plugged_in 0
}

impl Default for StayOnMode {
    fn default() -> Self {
        StayOnMode::Always
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub stay_awake: bool,
    pub stay_on_mode: StayOnMode,
    pub turn_off_physical_screen: bool,
    pub always_on_top: bool,
    pub fullscreen: bool,
    pub enable_audio: bool,
    pub max_fps: u32,
    pub max_size: u32,
    pub bitrate_mbps: u32,
    pub default_download_dir: String,
    pub auto_stay_awake_on_connect: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        let default_download = dirs_next_download().unwrap_or_else(|| "C:\\Users\\Public\\Downloads".to_string());
        Self {
            stay_awake: true,
            stay_on_mode: StayOnMode::Always,
            turn_off_physical_screen: false,
            always_on_top: false,
            fullscreen: false,
            enable_audio: true,
            max_fps: 60,
            max_size: 0, // 0 = original resolution
            bitrate_mbps: 16,
            default_download_dir: default_download,
            auto_stay_awake_on_connect: true,
        }
    }
}

fn dirs_next_download() -> Option<String> {
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let dl = format!("{}\\Downloads", userprofile);
        if std::path::Path::new(&dl).exists() {
            return Some(dl);
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub serial: String,
    pub state: String,
    pub model: String,
    pub product: String,
    pub battery_level: Option<u8>,
    pub battery_charging: bool,
    pub android_version: Option<String>,
    pub stay_on_active: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RemoteFile {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: String,
    pub permissions: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferStatus {
    Queued,
    Transferring,
    Completed,
    Failed(String),
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TransferItem {
    pub id: u64,
    pub is_upload: bool,
    pub is_apk_install: bool,
    pub local_path: String,
    pub remote_path: String,
    pub file_name: String,
    pub file_size: u64,
    pub status: TransferStatus,
    pub message: String,
}

use std::path::PathBuf;
use crate::process::{find_binary, run_hidden_command};
use crate::types::{DeviceInfo, StayOnMode};

pub fn get_adb_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let bin_name = "adb.exe";
    #[cfg(not(target_os = "windows"))]
    let bin_name = "adb";

    find_binary(bin_name).ok_or_else(|| {
        "ADB introuvable. Veuillez vérifier que adb.exe est dans le dossier bin/ ou dans le PATH.".to_string()
    })
}

pub fn list_devices() -> Result<Vec<DeviceInfo>, String> {
    let adb_path = get_adb_path()?;
    let output = run_hidden_command(&adb_path, &["devices", "-l"])
        .map_err(|e| format!("Erreur exécution adb devices: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut devices = Vec::new();

    for line in stdout.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        let serial = parts[0].to_string();
        let state = if parts.len() > 1 {
            parts[1].to_string()
        } else {
            "unknown".to_string()
        };

        if state != "device" {
            devices.push(DeviceInfo {
                serial: serial.clone(),
                state,
                model: "Inconnu".to_string(),
                product: "".to_string(),
                battery_level: None,
                battery_charging: false,
                android_version: None,
                stay_on_active: false,
            });
            continue;
        }

        let mut model = String::new();
        let mut product = String::new();

        for part in &parts[2..] {
            if let Some(m) = part.strip_prefix("model:") {
                model = m.replace('_', " ");
            } else if let Some(p) = part.strip_prefix("product:") {
                product = p.to_string();
            }
        }

        if model.is_empty() {
            model = serial.clone();
        }

        let (battery_level, battery_charging) = get_battery_info(&adb_path, &serial);
        let android_version = get_android_version(&adb_path, &serial);
        let stay_on_active = is_stay_on_active(&adb_path, &serial);

        devices.push(DeviceInfo {
            serial,
            state,
            model,
            product,
            battery_level,
            battery_charging,
            android_version,
            stay_on_active,
        });
    }

    Ok(devices)
}

fn get_battery_info(adb_path: &PathBuf, serial: &str) -> (Option<u8>, bool) {
    if let Ok(output) = run_hidden_command(adb_path, &["-s", serial, "shell", "dumpsys", "battery"]) {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut level = None;
        let mut charging = false;

        for line in text.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("level:") {
                if let Ok(parsed) = val.trim().parse::<u8>() {
                    level = Some(parsed);
                }
            } else if let Some(val) = line.strip_prefix("status:") {
                let status_val = val.trim();
                // 2 = BATTERY_STATUS_CHARGING, 5 = BATTERY_STATUS_FULL
                if status_val == "2" || status_val == "5" {
                    charging = true;
                }
            }
        }
        (level, charging)
    } else {
        (None, false)
    }
}

fn get_android_version(adb_path: &PathBuf, serial: &str) -> Option<String> {
    if let Ok(output) = run_hidden_command(
        adb_path,
        &["-s", serial, "shell", "getprop", "ro.build.version.release"],
    ) {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

pub fn is_stay_on_active(adb_path: &PathBuf, serial: &str) -> bool {
    if let Ok(output) = run_hidden_command(
        adb_path,
        &[
            "-s",
            serial,
            "shell",
            "settings",
            "get",
            "global",
            "stay_on_while_plugged_in",
        ],
    ) {
        let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
        val == "7" || val == "2" || val == "3" || val == "1"
    } else {
        false
    }
}

pub fn set_stay_on(serial: &str, mode: &StayOnMode) -> Result<(), String> {
    let adb_path = get_adb_path()?;
    match mode {
        StayOnMode::Always => {
            // Activer le mode stayon et forcer la valeur globale à 7 (AC + USB + Wireless)
            let _ = run_hidden_command(&adb_path, &["-s", serial, "shell", "svc", "power", "stayon", "true"]);
            run_hidden_command(
                &adb_path,
                &[
                    "-s",
                    serial,
                    "shell",
                    "settings",
                    "put",
                    "global",
                    "stay_on_while_plugged_in",
                    "7",
                ],
            )
            .map_err(|e| format!("Erreur activation Stay On: {}", e))?;
        }
        StayOnMode::UsbOnly => {
            let _ = run_hidden_command(&adb_path, &["-s", serial, "shell", "svc", "power", "stayon", "usb"]);
            run_hidden_command(
                &adb_path,
                &[
                    "-s",
                    serial,
                    "shell",
                    "settings",
                    "put",
                    "global",
                    "stay_on_while_plugged_in",
                    "2",
                ],
            )
            .map_err(|e| format!("Erreur activation Stay On USB: {}", e))?;
        }
        StayOnMode::SystemDefault => {
            let _ = run_hidden_command(&adb_path, &["-s", serial, "shell", "svc", "power", "stayon", "false"]);
            run_hidden_command(
                &adb_path,
                &[
                    "-s",
                    serial,
                    "shell",
                    "settings",
                    "put",
                    "global",
                    "stay_on_while_plugged_in",
                    "0",
                ],
            )
            .map_err(|e| format!("Erreur désactivation Stay On: {}", e))?;
        }
    }
    Ok(())
}

pub fn set_screen_timeout_ms(serial: &str, timeout_ms: u64) -> Result<(), String> {
    let adb_path = get_adb_path()?;
    let ms_str = timeout_ms.to_string();
    run_hidden_command(
        &adb_path,
        &[
            "-s",
            serial,
            "shell",
            "settings",
            "put",
            "system",
            "screen_off_timeout",
            &ms_str,
        ],
    )
    .map_err(|e| format!("Erreur réglage délai extinction: {}", e))?;
    Ok(())
}

pub fn wake_screen(serial: &str) -> Result<(), String> {
    let adb_path = get_adb_path()?;
    run_hidden_command(&adb_path, &["-s", serial, "shell", "input", "keyevent", "224"])
        .map_err(|e| format!("Erreur réveil écran: {}", e))?;
    Ok(())
}

pub fn sleep_screen(serial: &str) -> Result<(), String> {
    let adb_path = get_adb_path()?;
    run_hidden_command(&adb_path, &["-s", serial, "shell", "input", "keyevent", "223"])
        .map_err(|e| format!("Erreur veille écran: {}", e))?;
    Ok(())
}

pub fn toggle_power(serial: &str) -> Result<(), String> {
    let adb_path = get_adb_path()?;
    run_hidden_command(&adb_path, &["-s", serial, "shell", "input", "keyevent", "26"])
        .map_err(|e| format!("Erreur bouton power: {}", e))?;
    Ok(())
}

pub fn send_key_event(serial: &str, keycode: &str) -> Result<(), String> {
    let adb_path = get_adb_path()?;
    run_hidden_command(&adb_path, &["-s", serial, "shell", "input", "keyevent", keycode])
        .map_err(|e| format!("Erreur envoi touche {}: {}", keycode, e))?;
    Ok(())
}

pub fn enable_tcpip(serial: &str, port: u16) -> Result<String, String> {
    let adb_path = get_adb_path()?;
    let port_str = port.to_string();
    let output = run_hidden_command(&adb_path, &["-s", serial, "tcpip", &port_str])
        .map_err(|e| format!("Erreur activation TCP/IP: {}", e))?;
    let msg = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(msg)
}

pub fn connect_wireless(address: &str) -> Result<String, String> {
    let adb_path = get_adb_path()?;
    let output = run_hidden_command(&adb_path, &["connect", address])
        .map_err(|e| format!("Erreur connexion WiFi: {}", e))?;
    let msg = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(msg)
}

pub fn disconnect_wireless(address: &str) -> Result<String, String> {
    let adb_path = get_adb_path()?;
    let output = run_hidden_command(&adb_path, &["disconnect", address])
        .map_err(|e| format!("Erreur déconnexion WiFi: {}", e))?;
    let msg = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(msg)
}

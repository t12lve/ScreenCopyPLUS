use std::path::Path;
use crate::adb::get_adb_path;
use crate::process::run_hidden_command;
use crate::types::RemoteFile;

pub struct StorageShortcut {
    pub name: &'static str,
    pub icon: &'static str,
    pub path: &'static str,
}

pub const SHORTCUTS: &[StorageShortcut] = &[
    StorageShortcut {
        name: "Téléchargements",
        icon: "📥",
        path: "/sdcard/Download",
    },
    StorageShortcut {
        name: "Photos (DCIM)",
        icon: "📸",
        path: "/sdcard/DCIM/Camera",
    },
    StorageShortcut {
        name: "Images",
        icon: "🖼️",
        path: "/sdcard/Pictures",
    },
    StorageShortcut {
        name: "Documents",
        icon: "📄",
        path: "/sdcard/Documents",
    },
    StorageShortcut {
        name: "Musique",
        icon: "🎵",
        path: "/sdcard/Music",
    },
    StorageShortcut {
        name: "Vidéos",
        icon: "🎬",
        path: "/sdcard/Movies",
    },
    StorageShortcut {
        name: "Racine SDCard",
        icon: "📁",
        path: "/sdcard",
    },
];

pub fn list_directory(serial: &str, remote_path: &str) -> Result<Vec<RemoteFile>, String> {
    let adb_path = get_adb_path()?;
    let path_arg = if remote_path.is_empty() {
        "/sdcard"
    } else {
        remote_path
    };

    let output = run_hidden_command(
        &adb_path,
        &[
            "-s",
            serial,
            "shell",
            "ls",
            "-laL",
            "--color=never",
            path_arg,
        ],
    )
    .map_err(|e| format!("Erreur parcours dossier: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("total ") {
            continue;
        }

        // Parse ls -laL format
        // Example: drwxrwx--x 4 root sdcard_rw 4096 2026-10-15 14:22 Download
        // Example: -rw-rw---- 1 root sdcard_rw 1245678 2026-10-16 09:30 photo.jpg
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 7 {
            continue;
        }

        let permissions = parts[0];
        let is_dir = permissions.starts_with('d');

        // Check if this line is . or ..
        let name_idx = if parts.len() >= 8 {
            // Check date/time indices
            // Standard android toolbox / toybox ls:
            // perms links owner group size date time name...
            7
        } else {
            6
        };

        let file_name = parts[name_idx..].join(" ");
        if file_name == "." || file_name == ".." {
            continue;
        }

        let size: u64 = if is_dir {
            0
        } else {
            parts.get(4).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0)
        };

        let modified = if parts.len() >= 7 {
            format!("{} {}", parts[5], parts[6])
        } else {
            "".to_string()
        };

        let full_path = if path_arg.ends_with('/') {
            format!("{}{}", path_arg, file_name)
        } else {
            format!("{}/{}", path_arg, file_name)
        };

        files.push(RemoteFile {
            name: file_name,
            path: full_path,
            is_dir,
            size,
            modified,
            permissions: permissions.to_string(),
        });
    }

    // Sort folders first, then alphabetical
    files.sort_by(|a, b| {
        if a.is_dir && !b.is_dir {
            std::cmp::Ordering::Less
        } else if !a.is_dir && b.is_dir {
            std::cmp::Ordering::Greater
        } else {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        }
    });

    Ok(files)
}

pub fn push_item(serial: &str, local_path: &Path, remote_dest_dir: &str) -> Result<String, String> {
    let adb_path = get_adb_path()?;
    let local_str = local_path.to_string_lossy().to_string();

    let output = run_hidden_command(&adb_path, &["-s", serial, "push", &local_str, remote_dest_dir])
        .map_err(|e| format!("Erreur envoi adb push: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Échec de l'envoi: {}", err.trim()));
    }

    let out = String::from_utf8_lossy(&output.stdout);
    Ok(out.trim().to_string())
}

pub fn pull_item(serial: &str, remote_path: &str, local_dest: &Path) -> Result<String, String> {
    let adb_path = get_adb_path()?;
    let dest_str = local_dest.to_string_lossy().to_string();

    let output = run_hidden_command(&adb_path, &["-s", serial, "pull", remote_path, &dest_str])
        .map_err(|e| format!("Erreur téléchargement adb pull: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Échec du téléchargement: {}", err.trim()));
    }

    let out = String::from_utf8_lossy(&output.stdout);
    Ok(out.trim().to_string())
}

pub fn install_apk(serial: &str, local_apk_path: &Path) -> Result<String, String> {
    let adb_path = get_adb_path()?;
    let local_str = local_apk_path.to_string_lossy().to_string();

    let output = run_hidden_command(&adb_path, &["-s", serial, "install", "-r", &local_str])
        .map_err(|e| format!("Erreur installation APK: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if stdout.contains("Success") {
        Ok("Application APK installée avec succès !".to_string())
    } else {
        Err(format!("Erreur APK: {} {}", stdout.trim(), stderr.trim()))
    }
}

pub fn delete_remote(serial: &str, remote_path: &str) -> Result<(), String> {
    let adb_path = get_adb_path()?;
    let output = run_hidden_command(&adb_path, &["-s", serial, "shell", "rm", "-rf", remote_path])
        .map_err(|e| format!("Erreur suppression: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Échec suppression: {}", err));
    }
    Ok(())
}

pub fn create_remote_folder(serial: &str, remote_path: &str) -> Result<(), String> {
    let adb_path = get_adb_path()?;
    let output = run_hidden_command(&adb_path, &["-s", serial, "shell", "mkdir", "-p", remote_path])
        .map_err(|e| format!("Erreur création dossier: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Échec création dossier: {}", err));
    }
    Ok(())
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} Go", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} Mo", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} Ko", bytes as f64 / KB as f64)
    } else {
        format!("{} o", bytes)
    }
}

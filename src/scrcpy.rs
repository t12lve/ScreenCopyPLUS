use std::path::PathBuf;
use std::process::Child;
use crate::process::{find_binary, spawn_hidden_command};
use crate::types::AppConfig;

pub fn get_scrcpy_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let bin_name = "scrcpy.exe";
    #[cfg(not(target_os = "windows"))]
    let bin_name = "scrcpy";

    find_binary(bin_name).ok_or_else(|| {
        "scrcpy.exe introuvable. Veuillez vérifier que scrcpy.exe est dans le dossier bin/ ou dans le PATH.".to_string()
    })
}

pub fn launch_mirroring(serial: &str, config: &AppConfig) -> Result<Child, String> {
    let scrcpy_path = get_scrcpy_path()?;
    let working_dir = scrcpy_path.parent();

    let mut args: Vec<String> = Vec::new();

    // Spécifier le numéro de série de l'appareil
    args.push("-s".to_string());
    args.push(serial.to_string());

    // Option: Écran toujours allumé pendant le mirroring
    if config.stay_awake {
        args.push("--stay-awake".to_string());
    }

    // Option: Éteindre l'écran physique pendant la session
    if config.turn_off_physical_screen {
        args.push("--turn-screen-off".to_string());
    }

    // Option: Toujours au premier plan
    if config.always_on_top {
        args.push("--always-on-top".to_string());
    }

    // Option: Plein écran
    if config.fullscreen {
        args.push("--fullscreen".to_string());
    }

    // Option: Audio
    if !config.enable_audio {
        args.push("--no-audio".to_string());
    }

    // Option: FPS maximal
    if config.max_fps > 0 {
        args.push("--max-fps".to_string());
        args.push(config.max_fps.to_string());
    }

    // Option: Résolution maximale
    if config.max_size > 0 {
        args.push("-m".to_string());
        args.push(config.max_size.to_string());
    }

    // Option: Bitrate
    if config.bitrate_mbps > 0 {
        args.push("-b".to_string());
        args.push(format!("{}M", config.bitrate_mbps));
    }

    let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    // Lancer scrcpy sans AUCUNE fenêtre de console (CREATE_NO_WINDOW)
    spawn_hidden_command(&scrcpy_path, &args_str, working_dir)
        .map_err(|e| format!("Erreur lors du lancement de ScreenCopy: {}", e))
}

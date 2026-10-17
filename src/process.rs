use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

pub const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn prepare_hidden_command<P: AsRef<Path>>(program: P) -> Command {
    let mut cmd = Command::new(program.as_ref());
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub fn run_hidden_command<P: AsRef<Path>>(program: P, args: &[&str]) -> Result<Output, std::io::Error> {
    let mut cmd = prepare_hidden_command(program);
    cmd.args(args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.output()
}

pub fn spawn_hidden_command<P: AsRef<Path>>(
    program: P,
    args: &[&str],
    current_dir: Option<&Path>,
) -> Result<Child, std::io::Error> {
    let mut cmd = prepare_hidden_command(program);
    cmd.args(args);
    if let Some(dir) = current_dir {
        cmd.current_dir(dir);
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.spawn()
}

pub fn find_binary(binary_name: &str) -> Option<PathBuf> {
    // 1. Check ./bin/<binary_name>
    let local_bin = PathBuf::from("bin").join(binary_name);
    if local_bin.exists() {
        return Some(local_bin);
    }

    // 2. Check next to current executable
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let next_to_exe = exe_dir.join(binary_name);
            if next_to_exe.exists() {
                return Some(next_to_exe);
            }
            let bin_subdir = exe_dir.join("bin").join(binary_name);
            if bin_subdir.exists() {
                return Some(bin_subdir);
            }
            // Check parent bin
            if let Some(parent) = exe_dir.parent() {
                let parent_bin = parent.join("bin").join(binary_name);
                if parent_bin.exists() {
                    return Some(parent_bin);
                }
                if let Some(grandparent) = parent.parent() {
                    let gp_bin = grandparent.join("bin").join(binary_name);
                    if gp_bin.exists() {
                        return Some(gp_bin);
                    }
                }
            }
        }
    }

    // 3. For adb, check Android SDK
    if binary_name.starts_with("adb") {
        if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
            let sdk_adb = PathBuf::from(localappdata)
                .join("Android")
                .join("Sdk")
                .join("platform-tools")
                .join(binary_name);
            if sdk_adb.exists() {
                return Some(sdk_adb);
            }
        }
    }

    // 4. Check system PATH
    which(binary_name)
}

fn which(binary_name: &str) -> Option<PathBuf> {
    if let Ok(path_var) = std::env::var("PATH") {
        for path_entry in std::env::split_paths(&path_var) {
            let candidate = path_entry.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
            #[cfg(target_os = "windows")]
            {
                let candidate_exe = path_entry.join(format!("{}.exe", binary_name));
                if candidate_exe.is_file() {
                    return Some(candidate_exe);
                }
            }
        }
    }
    None
}

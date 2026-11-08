//! Removes the agent script that opensecmon offered before this program on Windows (`opensecmon-agent.ps1` with
//! the scheduled task `opensecmon-agent` and its folder `%ProgramData%\opensecmon-agent`, as `Invoke-Uninstall` of
//! the script does), so a computer does not report twice. Runs after a successful enrollment as administrator.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The scheduled task and the folder of the script (see `contrib/opensecmon-agent.ps1` in opensecmon).
const TASK: &str = "opensecmon-agent";
const DIR: &str = "opensecmon-agent";

/// What was removed, for the message to the user.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Removed {
    pub paths: Vec<PathBuf>,
}

/// `%ProgramData%` of the real system.
pub fn root() -> PathBuf {
    crate::store::default_dir()
        .parent()
        .map_or_else(|| PathBuf::from(r"C:\ProgramData"), Path::to_path_buf)
}

/// Removes the script's folder below `program_data`; on the real system also its scheduled task first, so it does
/// not start again while the folder goes.
pub fn remove_old_script(program_data: &Path) -> io::Result<Removed> {
    let mut removed = Removed::default();
    if program_data == root() && delete_task() {
        removed
            .paths
            .push(PathBuf::from(format!(r"Task Scheduler\{TASK}")));
    }
    let dir = program_data.join(DIR);
    match std::fs::symlink_metadata(&dir) {
        // A link (junction) is removed itself, never what it points to.
        Ok(m) if m.file_type().is_symlink() => {
            std::fs::remove_dir(&dir).or_else(|_| std::fs::remove_file(&dir))?;
            removed.paths.push(dir);
        }
        Ok(m) if m.is_dir() => {
            std::fs::remove_dir_all(&dir)?;
            removed.paths.push(dir);
        }
        Ok(_) => {
            std::fs::remove_file(&dir)?;
            removed.paths.push(dir);
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(removed)
}

/// Deletes the scheduled task with `schtasks.exe` of the system; whether there was one.
fn delete_task() -> bool {
    let system = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Command::new(Path::new(&system).join(r"System32\schtasks.exe"))
        .args(["/Delete", "/TN", TASK, "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_of_the_script_is_removed_and_nothing_else() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        assert_eq!(remove_old_script(r).unwrap(), Removed::default());
        std::fs::create_dir(r.join(DIR)).unwrap();
        std::fs::write(r.join(DIR).join("auth-header"), "x").unwrap();
        std::fs::create_dir(r.join("opensecagent")).unwrap();
        let removed = remove_old_script(r).unwrap();
        assert_eq!(removed.paths, [r.join(DIR)]);
        assert!(!r.join(DIR).exists());
        assert!(r.join("opensecagent").is_dir());
    }

    #[test]
    fn a_junction_in_place_of_the_folder_is_removed_without_following_it() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let target = r.join("keep");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("file"), "x").unwrap();
        let made = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(r.join(DIR))
            .arg(&target)
            .output()
            .unwrap();
        assert!(made.status.success());
        let removed = remove_old_script(r).unwrap();
        assert_eq!(removed.paths, [r.join(DIR)]);
        assert!(!r.join(DIR).exists());
        assert!(target.join("file").exists());
    }
}

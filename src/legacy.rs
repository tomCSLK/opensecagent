//! Removes the agent script that opensecmon offered before this program (`opensecmon-agent.sh` with a systemd
//! timer), so a computer does not report twice. Runs after a successful enrollment as root.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where the script installed itself (see `contrib/opensecmon-agent.sh` in opensecmon).
const SETTINGS_DIR: &str = "etc/opensecmon-agent";
const PROGRAM: &str = "usr/local/sbin/opensecmon-agent";
const SERVICE: &str = "etc/systemd/system/opensecmon-agent.service";
const TIMER: &str = "etc/systemd/system/opensecmon-agent.timer";

/// What was removed, for the message to the user.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Removed {
    pub paths: Vec<PathBuf>,
}

/// The root of the real system.
pub fn root() -> PathBuf {
    PathBuf::from("/")
}

/// Removes the script below `root` (`/` on a real system; systemd is only asked there). Paths that are not there
/// are skipped; the first one that cannot be removed ends it with an error.
pub fn remove_old_script(root: &Path) -> io::Result<Removed> {
    let real = root == Path::new("/");
    let mut removed = Removed::default();
    let units = [root.join(TIMER), root.join(SERVICE)];
    if units.iter().any(|u| exists(u)) {
        if real {
            systemctl(&["disable", "--now", "opensecmon-agent.timer"]);
        }
        for unit in &units {
            remove_file(unit, &mut removed)?;
        }
        if real {
            systemctl(&["daemon-reload"]);
        }
    }
    let dir = root.join(SETTINGS_DIR);
    match std::fs::symlink_metadata(&dir) {
        // A link is removed itself, never what it points to.
        Ok(m) if m.is_dir() => {
            std::fs::remove_dir_all(&dir)?;
            removed.paths.push(dir);
        }
        Ok(_) => remove_file(&dir, &mut removed)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    remove_file(&root.join(PROGRAM), &mut removed)?;
    Ok(removed)
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn remove_file(path: &Path, removed: &mut Removed) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            removed.paths.push(path.to_path_buf());
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Asks systemd; a failure is not an error here (the unit files are removed anyway).
fn systemctl(args: &[&str]) {
    let _ = Command::new("systemctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_is_removed_and_nothing_else() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        assert_eq!(remove_old_script(r).unwrap(), Removed::default());

        for p in [SERVICE, TIMER, PROGRAM, "etc/opensecmon-agent/token"] {
            let p = r.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "x").unwrap();
        }
        let other = r.join("etc/systemd/system/other.service");
        std::fs::write(&other, "x").unwrap();
        let removed = remove_old_script(r).unwrap();
        assert_eq!(removed.paths.len(), 4, "{removed:?}");
        for p in [SERVICE, TIMER, PROGRAM, SETTINGS_DIR] {
            assert!(!exists(&r.join(p)), "{p}");
        }
        assert!(other.exists());
        assert!(r.join("usr/local/sbin").is_dir());
    }

    #[test]
    fn a_link_in_place_of_the_settings_is_removed_without_following_it() {
        let root = tempfile::tempdir().unwrap();
        let r = root.path();
        let target = r.join("keep");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("file"), "x").unwrap();
        std::fs::create_dir_all(r.join("etc")).unwrap();
        std::os::unix::fs::symlink(&target, r.join(SETTINGS_DIR)).unwrap();
        let removed = remove_old_script(r).unwrap();
        assert_eq!(removed.paths, [r.join(SETTINGS_DIR)]);
        assert!(target.join("file").exists());
    }
}

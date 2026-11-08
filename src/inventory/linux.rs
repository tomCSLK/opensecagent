//! The inventory of Debian, Ubuntu and systems based on them: what the script `opensecmon-agent.sh` reads, plus
//! the Flatpaks installed for single users.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use super::{Inventory, InventoryError, Os, flatpak, snap};

/// opensecmon takes at most 16 KB of os-release.
const OS_RELEASE_MAX: u64 = 16 * 1024;
/// opensecmon takes at most 8 MB per report; a package list is far below that.
const DPKG_MAX: u64 = 8 * 1024 * 1024;
/// The format of the package lines, the same as in the inventory script of opensecmon.
const DPKG_FORMAT: &str =
    "${db:Status-Abbrev}\\t${Package}\\t${Version}\\t${source:Package}\\t${source:Version}\\n";

pub fn collect() -> Result<Inventory, InventoryError> {
    let os_release = read_os_release(Path::new("/etc/os-release"))
        .or_else(|_| read_os_release(Path::new("/usr/lib/os-release")))?;
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|k| k.trim().to_owned())
        .filter(|k| is_kernel_release(k));
    let reboot_required = kernel.is_some() && Path::new("/var/run/reboot-required").exists();
    Ok(Inventory {
        hostname: super::hostname(),
        os: Os {
            family: "linux",
            os_release: Some(os_release),
            kernel,
            reboot_required: Some(reboot_required),
            ubuntu_pro: Some(has_ubuntu_pro(Path::new("/etc/apt/sources.list.d"))),
            arch: std::env::consts::ARCH.to_owned(),
            ..Os::default()
        },
        dpkg: Some(dpkg_packages()?),
        items: super::sorted({
            let mut items = snap::snaps()?;
            items.extend(flatpak::apps()?);
            items
        }),
    })
}

fn read_os_release(path: &Path) -> Result<String, InventoryError> {
    let err = |e: String| InventoryError::Read("/etc/os-release", e);
    let file = std::fs::File::open(path).map_err(|e| err(e.to_string()))?;
    let mut bytes = Vec::new();
    file.take(OS_RELEASE_MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| err(e.to_string()))?;
    if bytes.len() as u64 > OS_RELEASE_MAX {
        return Err(err("larger than 16 KB".into()));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Whether a text looks like a kernel release as `uname -r` gives it; opensecmon refuses anything else.
fn is_kernel_release(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-' | b'_' | b'~'))
}

/// Ubuntu Pro: the sources of esm-infra or esm-apps are set up.
fn has_ubuntu_pro(sources: &Path) -> bool {
    std::fs::read_dir(sources).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("ubuntu-esm-infra.") || name.starts_with("ubuntu-esm-apps.")
        })
    })
}

/// The package list of dpkg.
fn dpkg_packages() -> Result<String, InventoryError> {
    match run("dpkg-query", &["-W", "-f", DPKG_FORMAT], DPKG_MAX) {
        Ok(Some(out)) => Ok(out),
        Ok(None) => Err(InventoryError::Read(
            "the package list",
            "dpkg-query is missing: only Debian, Ubuntu and systems based on them are supported"
                .into(),
        )),
        Err(e) => Err(InventoryError::Read("the package list (dpkg-query)", e)),
    }
}

/// Runs a program of the system with the C locale and reads at most `max` bytes of its output; `None` if the
/// program is not installed. Its output is data: it is only parsed, never run.
pub(super) fn run(program: &str, args: &[&str], max: u64) -> Result<Option<String>, String> {
    let mut child = match Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .env_remove("LANGUAGE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("piped")
        .take(max + 1)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() as u64 > max {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    read.map_err(|e| e.to_string())?;
    if bytes.len() as u64 > max {
        return Err(format!(
            "the output of {program} is larger than {max} bytes"
        ));
    }
    if !status.success() {
        return Err(format!("{program} ended with {status}"));
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_releases_are_checked_as_opensecmon_does() {
        assert!(is_kernel_release("6.8.0-40-generic"));
        assert!(is_kernel_release("6.18.60+rpt-rpi-v8"));
        assert!(!is_kernel_release(""));
        assert!(!is_kernel_release("6.8 $(reboot)"));
        assert!(!is_kernel_release(&"1".repeat(101)));
    }

    #[test]
    fn ubuntu_pro_is_seen_by_its_sources() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!has_ubuntu_pro(dir.path()));
        std::fs::write(dir.path().join("ubuntu-esm-apps.sources.disabled-x"), "").unwrap();
        assert!(has_ubuntu_pro(dir.path()));
        assert!(!has_ubuntu_pro(&dir.path().join("missing")));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ubuntu-esm-infra.list"), "").unwrap();
        assert!(has_ubuntu_pro(dir.path()));
    }

    #[test]
    fn os_release_is_limited() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("os-release");
        std::fs::write(&path, "ID=debian\nVERSION_ID=\"13\"\n").unwrap();
        assert_eq!(
            read_os_release(&path).unwrap(),
            "ID=debian\nVERSION_ID=\"13\"\n"
        );
        std::fs::write(&path, vec![b'#'; 17 * 1024]).unwrap();
        assert!(read_os_release(&path).is_err());
        assert!(read_os_release(&dir.path().join("missing")).is_err());
    }

    /// On a Debian or Ubuntu machine (like the developer's), the real inventory.
    #[test]
    fn the_inventory_of_this_machine() {
        if !Path::new("/usr/bin/dpkg-query").exists() || !Path::new("/etc/os-release").exists() {
            return;
        }
        use std::os::unix::fs::MetadataExt;
        let root = std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0);
        let inv = match collect() {
            Ok(inv) => inv,
            // Without root the Flatpaks of other users cannot be read (CI runners have a second home): the report
            // must wait then.
            Err(InventoryError::Read("the Flatpaks of the users", _)) if !root => return,
            Err(e) => panic!("{e}"),
        };
        assert_eq!(inv.os.family, "linux");
        assert!(inv.os.os_release.unwrap().contains("ID="));
        for item in &inv.items {
            assert!(["snap", "flatpak"].contains(&item.kind), "{item:?}");
        }
        let dpkg = inv.dpkg.unwrap();
        let first = dpkg.lines().next().unwrap();
        assert_eq!(first.split('\t').count(), 5, "{first}");
        assert!(dpkg.lines().any(|l| l.contains("\tdpkg\t")));
    }
}

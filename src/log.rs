//! The lines of the service. On Linux and in a terminal they go to standard output (systemd adds the time and keeps
//! them in the journal); the Windows service has no output, so there they go to `opensecagent.log` in the agent's
//! folder, with the time. A full file (1 MB) becomes `opensecagent.log.old`, replacing the one before.

use std::fmt::Arguments;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Size at which the file is started anew.
const MAX: u64 = 1024 * 1024;

static FILE: OnceLock<PathBuf> = OnceLock::new();
static WRITING: Mutex<()> = Mutex::new(());

/// From now on the lines go to this file (the Windows service). Only once; later calls change nothing.
#[cfg(windows)]
pub fn to_file(path: PathBuf) {
    let _ = FILE.set(path);
}

/// A line of the service: standard output, or the file.
pub fn line(args: Arguments<'_>) {
    match FILE.get() {
        Some(path) => append(path, MAX, crate::agent::now(), args),
        None => println!("{args}"),
    }
}

/// A note or warning beside the output of a command: standard error (which does not mix with a report on standard
/// output), or the file.
pub fn note(args: Arguments<'_>) {
    match FILE.get() {
        Some(path) => append(path, MAX, crate::agent::now(), args),
        None => eprintln!("{args}"),
    }
}

/// Appends a line with the time. Errors are dropped: there is no other place to report them.
fn append(path: &Path, max: u64, now: u64, args: Arguments<'_>) {
    let _guard = WRITING.lock().unwrap_or_else(|e| e.into_inner());
    if fs::metadata(path).is_ok_and(|m| m.len() >= max) {
        let mut old = path.as_os_str().to_owned();
        old.push(".old");
        let _ = fs::rename(path, old);
    }
    if let Ok(mut file) = fs::OpenOptions::new().append(true).create(true).open(path) {
        let _ = writeln!(file, "{} {args}", crate::date_time(now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_carry_the_time_and_a_full_file_is_started_anew() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opensecagent.log");
        let old = dir.path().join("opensecagent.log.old");
        append(&path, 60, 0, format_args!("first"));
        append(&path, 60, 61, format_args!("second {}", 2));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "1970-01-01 00:00:00 UTC first\n1970-01-01 00:01:01 UTC second 2\n"
        );
        assert!(!old.exists());
        // Over the size: the file moves to .old (replacing an older one), a new one begins.
        append(&path, 60, 62, format_args!("third"));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "1970-01-01 00:01:02 UTC third\n"
        );
        assert!(fs::read_to_string(&old).unwrap().contains("second 2"));
        for n in 0..3 {
            append(&path, 60, 63, format_args!("line {n}"));
        }
        assert!(fs::read_to_string(&old).unwrap().contains("third"));
        assert!(!fs::read_to_string(&old).unwrap().contains("second"));
    }
}

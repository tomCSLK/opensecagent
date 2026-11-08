//! "Enroll with opensecmon" in the start menu on Windows. Its manifest asks for administrator rights (UAC, see
//! `build.rs`); then it starts `opensecagent.exe enroll-dialog` from its own folder without a console window and
//! waits for it. A program of its own, because a console program like opensecagent.exe would open a console window
//! from the start menu.
//!
//! Copyright (C) 2026 Thomas Cieslik. Licensed under the GNU Affero General Public License, version 3 or later.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    use std::os::windows::process::CommandExt;
    use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

    /// Starts a console program without a console window.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let Ok(me) = std::env::current_exe() else {
        return ExitCode::FAILURE;
    };
    // The dialog is another process; it may come to the front.
    // SAFETY: no pointers.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    match std::process::Command::new(me.with_file_name("opensecagent.exe"))
        .arg("enroll-dialog")
        .creation_flags(CREATE_NO_WINDOW)
        .status()
    {
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("opensecagent-enroll is for Windows; elsewhere use `opensecagent enroll-dialog`");
    ExitCode::FAILURE
}

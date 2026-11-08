//! The enrollment dialog for Linux desktops ("Enroll with opensecmon" in the start menu): takes the enrollment
//! code from the clipboard or lets the user paste it, then enrolls through `pkexec` (the user confirms with an
//! administrator password). Runs as the user; only `opensecagent enroll` runs as root.
//!
//! The dialogs are zenity (GNOME and most desktops) or kdialog (KDE). The code never goes on a command line, where
//! other users could see it in the process list.

use std::io::{Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::code;
use crate::texts::{self, Lang, Text, offered_code, plain};

/// The code is about 100 characters; anything much longer in the clipboard is something else.
const CLIPBOARD_MAX: u64 = 4096;
/// A clipboard tool that does not answer in this time is given up.
const CLIPBOARD_TIMEOUT: Duration = Duration::from_secs(3);
/// The answer of `opensecagent enroll --for-dialog` is one short line.
const ANSWER_MAX: u64 = 64 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Toolkit {
    Zenity,
    Kdialog,
}

/// Shows the dialog until the computer is enrolled or the user cancels. Returns whether it was enrolled.
pub fn run() -> Result<bool, String> {
    let lang = lang(|k| std::env::var(k).ok());
    let kit = toolkit().ok_or_else(|| {
        "neither zenity nor kdialog is installed: enroll in a terminal with `sudo opensecagent enroll`".to_owned()
    })?;
    let ui = Ui { lang, kit };
    let program = std::env::current_exe().map_err(|e| e.to_string())?;

    let mut code_text =
        offered_code(clipboard_code()).filter(|(_, address)| ui.use_clipboard(address));
    loop {
        let text = match code_text.take() {
            Some((text, _)) => text,
            None => match ui.entry(ui.t(Text::Paste)) {
                // OK with an empty field: the code may have been copied after the dialog opened.
                Some(text) if text.trim().is_empty() => match offered_code(clipboard_code()) {
                    Some((text, address)) if ui.use_clipboard(&address) => text,
                    Some(_) => continue,
                    None => {
                        ui.error(ui.t(Text::NoCodeInClipboard));
                        continue;
                    }
                },
                Some(text) => text,
                None => return Ok(false),
            },
        };
        let decoded = match code::decode(&text) {
            Ok(c) => c,
            Err(code::CodeError::Prefix) => {
                ui.error(ui.t(Text::NotACode));
                continue;
            }
            Err(code::CodeError::Damaged) => {
                ui.error(ui.t(Text::Damaged));
                continue;
            }
        };
        let progress = ui.progress(
            &ui.t(Text::Working)
                .replace("{address}", &plain(&decoded.address)),
        );
        let answer = enroll(&program, text.trim());
        if let Some(mut p) = progress {
            let _ = p.kill();
            let _ = p.wait();
        }
        match answer {
            Answer::Enrolled {
                device,
                old_script_removed,
                first_report,
            } => {
                let mut msg = ui.t(Text::Done).replace("{device}", &plain(&device));
                if !first_report {
                    msg.push_str("\n\n");
                    msg.push_str(ui.t(Text::FirstReportLater));
                }
                if old_script_removed {
                    msg.push_str("\n\n");
                    msg.push_str(ui.t(Text::OldScript));
                }
                ui.info(&msg);
                return Ok(true);
            }
            Answer::Cancelled => return Ok(false),
            Answer::Failed { kind, message } => {
                let msg = match ui.failure(&kind) {
                    Some(text) => text.replace("{address}", &plain(&decoded.address)),
                    None => format!("{}\n\n{}", ui.t(Text::Other), plain(&message)),
                };
                ui.error(&msg);
            }
        }
    }
}

/// What `pkexec opensecagent enroll --for-dialog` came to.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    Enrolled {
        device: String,
        old_script_removed: bool,
        first_report: bool,
    },
    /// The user closed the password prompt.
    Cancelled,
    Failed {
        kind: String,
        message: String,
    },
}

fn enroll(program: &std::path::Path, code: &str) -> Answer {
    let child = Command::new("pkexec")
        .arg(program)
        .args(["enroll", "--for-dialog"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            return Answer::Failed {
                kind: "no_pkexec".into(),
                message: e.to_string(),
            };
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(code.as_bytes());
    }
    let mut out = Vec::new();
    let _ = child
        .stdout
        .take()
        .expect("piped")
        .take(ANSWER_MAX)
        .read_to_end(&mut out);
    let status = child.wait().ok();
    answer(
        status.and_then(|s| s.code()),
        &String::from_utf8_lossy(&out),
    )
}

/// Reads the answer: the last line of JSON, or the exit code of pkexec (126: the prompt was closed, 127: not
/// authorized).
fn answer(exit: Option<i32>, out: &str) -> Answer {
    let line = out.lines().rev().find(|l| l.starts_with('{'));
    let value: Option<serde_json::Value> = line.and_then(|l| serde_json::from_str(l).ok());
    let text = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or_default().to_owned();
    match (value, exit) {
        (Some(v), _) if v["ok"] == true => Answer::Enrolled {
            device: text(&v, "device"),
            old_script_removed: v["old_script_removed"] == true,
            first_report: v["first_report"] == true,
        },
        (Some(v), _) => Answer::Failed {
            kind: text(&v, "error"),
            message: text(&v, "message"),
        },
        (None, Some(126)) => Answer::Cancelled,
        (None, Some(127)) => Answer::Failed {
            kind: "not_authorized".into(),
            message: String::new(),
        },
        (None, _) => Answer::Failed {
            kind: "other".into(),
            message: "no answer from opensecagent".into(),
        },
    }
}

/// The language of the desktop session: German or English.
fn lang(var: impl Fn(&str) -> Option<String>) -> Lang {
    let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| var(k))
        .find(|v| !v.is_empty())
        .unwrap_or_default();
    if value.starts_with("de") {
        Lang::De
    } else {
        Lang::En
    }
}

fn toolkit() -> Option<Toolkit> {
    let kde =
        std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_uppercase().contains("KDE"));
    let order = if kde {
        [Toolkit::Kdialog, Toolkit::Zenity]
    } else {
        [Toolkit::Zenity, Toolkit::Kdialog]
    };
    order.into_iter().find(|kit| {
        let name = match kit {
            Toolkit::Zenity => "zenity",
            Toolkit::Kdialog => "kdialog",
        };
        installed(name)
    })
}

fn installed(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// A code from the clipboard, if one of the clipboard tools is installed and it holds one.
fn clipboard_code() -> Option<String> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let mut tools: Vec<(&str, &[&str])> = Vec::new();
    if wayland {
        tools.push(("wl-paste", &["--no-newline", "--type", "text/plain"]));
    }
    tools.push(("xclip", &["-out", "-selection", "clipboard"]));
    tools.push(("xsel", &["--clipboard", "--output"]));
    tools
        .into_iter()
        .filter(|(name, _)| installed(name))
        .find_map(|(name, args)| {
            let text = read_with_timeout(name, args)?;
            let text = text.trim();
            text.get(..code::PREFIX.len())
                .is_some_and(|p| p.eq_ignore_ascii_case(code::PREFIX))
                .then(|| text.to_owned())
        })
}

/// The output of a small program, or nothing if it fails, says too much or does not finish in time.
fn read_with_timeout(program: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = (&mut stdout)
            .take(CLIPBOARD_MAX + 1)
            .read_to_end(&mut bytes);
        bytes
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < CLIPBOARD_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let bytes = reader.join().ok()?;
    (status.success() && bytes.len() as u64 <= CLIPBOARD_MAX)
        .then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// zenity reads the texts as Pango markup.
fn pango(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

struct Ui {
    lang: Lang,
    kit: Toolkit,
}

impl Ui {
    fn t(&self, text: Text) -> &'static str {
        texts::t(self.lang, text)
    }

    /// The message for a kind of error from `opensecagent enroll --for-dialog`.
    fn failure(&self, kind: &str) -> Option<&'static str> {
        texts::failure(self.lang, kind)
    }

    fn command(&self) -> Command {
        let mut c = match self.kit {
            Toolkit::Zenity => Command::new("zenity"),
            Toolkit::Kdialog => Command::new("kdialog"),
        };
        c.arg(format!("--title={}", self.t(Text::Title)));
        c
    }

    fn text(&self, s: &str) -> String {
        match self.kit {
            Toolkit::Zenity => pango(s),
            Toolkit::Kdialog => s.to_owned(),
        }
    }

    fn status(mut c: Command) -> Option<ExitStatus> {
        c.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()
    }

    /// Asks whether to enroll with the code found in the clipboard.
    fn use_clipboard(&self, address: &str) -> bool {
        self.question(
            &self
                .t(Text::Clipboard)
                .replace("{address}", &plain(address)),
        )
    }

    fn question(&self, text: &str) -> bool {
        let mut c = self.command();
        match self.kit {
            Toolkit::Zenity => c.args([
                "--question".to_owned(),
                "--width=460".to_owned(),
                format!("--text={}", self.text(text)),
                format!("--ok-label={}", self.t(Text::Use)),
                format!("--cancel-label={}", self.t(Text::AnotherCode)),
            ]),
            Toolkit::Kdialog => c.args([
                format!("--yes-label={}", self.t(Text::Use)),
                format!("--no-label={}", self.t(Text::AnotherCode)),
                "--yesno".to_owned(),
                self.text(text),
            ]),
        };
        Self::status(c).is_some_and(|s| s.success())
    }

    /// The text the user entered, `None` if the dialog was cancelled.
    fn entry(&self, text: &str) -> Option<String> {
        let mut c = self.command();
        match self.kit {
            Toolkit::Zenity => c.args([
                "--entry".to_owned(),
                "--width=560".to_owned(),
                format!("--text={}", self.text(text)),
                format!("--ok-label={}", self.t(Text::Enroll)),
            ]),
            Toolkit::Kdialog => c.args(["--inputbox".to_owned(), self.text(text)]),
        };
        let out = c.stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    fn error(&self, text: &str) {
        let mut c = self.command();
        match self.kit {
            Toolkit::Zenity => c.args([
                "--error".to_owned(),
                "--width=460".to_owned(),
                format!("--text={}", self.text(text)),
            ]),
            Toolkit::Kdialog => c.args(["--error".to_owned(), self.text(text)]),
        };
        Self::status(c);
    }

    fn info(&self, text: &str) {
        let mut c = self.command();
        match self.kit {
            Toolkit::Zenity => c.args([
                "--info".to_owned(),
                "--width=460".to_owned(),
                format!("--text={}", self.text(text)),
            ]),
            Toolkit::Kdialog => c.args(["--msgbox".to_owned(), self.text(text)]),
        };
        Self::status(c);
    }

    /// A window that shows the enrollment is running; closed by killing it.
    fn progress(&self, text: &str) -> Option<Child> {
        let mut c = self.command();
        match self.kit {
            Toolkit::Zenity => c.args([
                "--progress".to_owned(),
                "--pulsate".to_owned(),
                "--no-cancel".to_owned(),
                "--width=460".to_owned(),
                format!("--text={}", self.text(text)),
            ]),
            Toolkit::Kdialog => c.args([
                "--passivepopup".to_owned(),
                self.text(text),
                "10".to_owned(),
            ]),
        };
        c.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_language_comes_from_the_session() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert_eq!(lang(env(&[("LANG", "de_DE.UTF-8")])), Lang::De);
        assert_eq!(
            lang(env(&[("LANG", "de_DE.UTF-8"), ("LC_ALL", "C")])),
            Lang::En
        );
        assert_eq!(
            lang(env(&[("LC_ALL", ""), ("LANG", "de_AT.UTF-8")])),
            Lang::De
        );
        assert_eq!(
            lang(env(&[("LC_MESSAGES", "en_US.UTF-8"), ("LANG", "de_DE")])),
            Lang::En
        );
        assert_eq!(lang(env(&[])), Lang::En);
    }

    #[test]
    fn answers_of_the_enrollment_are_read() {
        assert_eq!(
            answer(
                Some(0),
                "noise\n{\"ok\":true,\"device\":\"laptop\",\"address\":\"a:8444\",\"old_script_removed\":true,\"first_report\":false}\n"
            ),
            Answer::Enrolled {
                device: "laptop".into(),
                old_script_removed: true,
                first_report: false
            }
        );
        assert_eq!(
            answer(
                Some(1),
                "{\"ok\":false,\"error\":\"code_invalid\",\"message\":\"m\"}"
            ),
            Answer::Failed {
                kind: "code_invalid".into(),
                message: "m".into()
            }
        );
        assert_eq!(answer(Some(126), ""), Answer::Cancelled);
        assert!(
            matches!(answer(Some(127), ""), Answer::Failed { kind, .. } if kind == "not_authorized")
        );
        assert!(matches!(answer(None, "garbage"), Answer::Failed { kind, .. } if kind == "other"));
    }

    #[test]
    fn every_error_kind_of_the_agent_has_words() {
        for lang in [Lang::De, Lang::En] {
            let ui = Ui {
                lang,
                kit: Toolkit::Zenity,
            };
            for kind in [
                "code_damaged",
                "code_invalid",
                "network_not_allowed",
                "unreachable",
                "untrusted",
                "wait",
                "unsupported_system",
                "not_authorized",
                "no_pkexec",
            ] {
                assert!(ui.failure(kind).is_some(), "{kind}");
            }
            assert!(ui.failure("other").is_none());
        }
    }

    #[test]
    fn outside_text_cannot_become_markup() {
        assert_eq!(plain("<b>pc</b>\u{7}"), "bpc/b");
        assert_eq!(pango("a & <b>"), "a &amp; &lt;b&gt;");
    }
}

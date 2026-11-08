//! The enrollment dialog on Windows: "Enroll with opensecmon" in the start menu, and at the end of the installer.
//! `opensecagent-enroll.exe` asks for administrator rights (UAC) and starts `opensecagent.exe enroll-dialog` without
//! a console window.
//!
//! Task dialogs of Windows, no text field: the code is too long to type, so the user copies it in opensecmon and
//! clicks "Enroll", and the dialog takes it from the clipboard (it already says so when a code is there). While the
//! enrollment runs in a thread, a dialog with a moving bar shows; then it says how it went, in the user's language,
//! with the texts of the Linux dialog.

use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ::windows::Win32::Foundation::{
    CloseHandle, HANDLE, HGLOBAL, HWND, LPARAM, S_FALSE, S_OK, WPARAM,
};
use ::windows::Win32::Globalization::GetUserDefaultUILanguage;
use ::windows::Win32::Security::{
    GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
};
use ::windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard,
};
use ::windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use ::windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use ::windows::Win32::UI::Controls::{
    TASKDIALOG_BUTTON, TASKDIALOG_COMMON_BUTTON_FLAGS, TASKDIALOG_FLAGS, TASKDIALOG_NOTIFICATIONS,
    TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TD_ERROR_ICON, TD_INFORMATION_ICON, TDCBF_CANCEL_BUTTON,
    TDCBF_CLOSE_BUTTON, TDE_CONTENT, TDF_CALLBACK_TIMER, TDF_SHOW_MARQUEE_PROGRESS_BAR,
    TDM_CLICK_BUTTON, TDM_ENABLE_BUTTON, TDM_SET_ELEMENT_TEXT, TDM_SET_PROGRESS_BAR_MARQUEE,
    TDN_BUTTON_CLICKED, TDN_CREATED, TDN_TIMER, TaskDialogIndirect,
};
use ::windows::Win32::UI::WindowsAndMessaging::{SendMessageW, SetForegroundWindow};
use ::windows::core::{HRESULT, HSTRING, PCWSTR};

use crate::agent::Agent;
use crate::code;
use crate::store::Store;
use crate::texts::{self, Lang, Text, offered_code, plain};

/// Clipboard format of Unicode text.
const CF_UNICODETEXT: u32 = 13;
/// The code is about 100 characters; anything much longer in the clipboard is something else.
const CLIPBOARD_MAX_CHARS: usize = 4096;
/// Button ids: Cancel of Windows, and our own.
const IDCANCEL: i32 = 2;
const ID_ENROLL: i32 = 100;
const ID_TRY_AGAIN: i32 = 101;

/// What the enrollment came to: the result, or the kind of error and its message.
type Outcome = Result<crate::DialogEnrollment, (String, String)>;

/// Shows the dialog until the computer is enrolled or the user cancels.
pub fn run(store: Store, default_dir: bool) -> ExitCode {
    let lang = lang();
    if default_dir && !elevated() {
        message(lang, TD_ERROR_ICON, texts::t(lang, Text::NeedsAdmin));
        return ExitCode::FAILURE;
    }
    loop {
        let Some(text) = ask(lang) else {
            return ExitCode::from(2);
        };
        let decoded = match code::decode(&text) {
            Ok(c) => c,
            Err(code::CodeError::Prefix) => {
                message(lang, TD_ERROR_ICON, texts::t(lang, Text::NotACode));
                continue;
            }
            Err(code::CodeError::Damaged) => {
                message(lang, TD_ERROR_ICON, texts::t(lang, Text::Damaged));
                continue;
            }
        };
        let address = plain(&decoded.address);
        match enroll(lang, &store, default_dir, text.trim().to_owned(), &address) {
            Ok(e) => {
                let mut msg = texts::t(lang, Text::Done).replace("{device}", &plain(&e.device));
                if !e.first_report {
                    msg.push_str("\n\n");
                    msg.push_str(texts::t(lang, Text::FirstReportLater));
                }
                if e.old_script_removed {
                    msg.push_str("\n\n");
                    msg.push_str(texts::t(lang, Text::OldScript));
                }
                message(lang, TD_INFORMATION_ICON, &msg);
                return ExitCode::SUCCESS;
            }
            Err((kind, msg)) => {
                let text = match (texts::failure(lang, &kind), kind.as_str()) {
                    (Some(text), _) => text.replace("{address}", &address),
                    (None, "files") => {
                        format!("{}\n\n{}", texts::t(lang, Text::Files), plain(&msg))
                    }
                    (None, _) => format!("{}\n\n{}", texts::t(lang, Text::Other), plain(&msg)),
                };
                if !try_again(lang, &text) {
                    return ExitCode::FAILURE;
                }
            }
        }
    }
}

/// The language of the user's Windows: German or English.
fn lang() -> Lang {
    // SAFETY: no arguments.
    let id = unsafe { GetUserDefaultUILanguage() };
    // The primary language is in the low 10 bits; 0x07 is German.
    if id & 0x3ff == 0x07 {
        Lang::De
    } else {
        Lang::En
    }
}

/// Whether the process runs with administrator rights (elevated).
fn elevated() -> bool {
    let mut token = HANDLE::default();
    // SAFETY: valid out pointers; the token is closed below.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some((&raw mut elevation).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// Text in the clipboard that starts like a code (it may still be damaged), or nothing.
fn clipboard_code() -> Option<String> {
    let text = clipboard_text()?;
    let text = text.trim();
    text.get(..code::PREFIX.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(code::PREFIX))
        .then(|| text.to_owned())
}

fn clipboard_text() -> Option<String> {
    // SAFETY: the clipboard is opened, read under GlobalLock and closed again; the length is bounded by GlobalSize.
    unsafe {
        // Another program may hold it for a moment.
        let mut open = false;
        for _ in 0..5 {
            if OpenClipboard(None).is_ok() {
                open = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if !open {
            return None;
        }
        let text = (|| {
            let handle = GetClipboardData(CF_UNICODETEXT).ok()?;
            let global = HGLOBAL(handle.0);
            let chars = GlobalSize(global) / 2;
            if chars == 0 || chars > CLIPBOARD_MAX_CHARS {
                return None;
            }
            let data = GlobalLock(global).cast::<u16>().cast_const();
            if data.is_null() {
                return None;
            }
            let all = std::slice::from_raw_parts(data, chars);
            let end = all.iter().position(|&c| c == 0).unwrap_or(chars);
            let text = String::from_utf16_lossy(&all[..end]);
            let _ = GlobalUnlock(global);
            Some(text)
        })();
        let _ = CloseClipboard();
        text
    }
}

/// The first dialog: what to do, or that a code is in the clipboard. Returns the text of the clipboard when the
/// user clicks "Enroll" with something like a code there; `None` if the user cancels.
fn ask(lang: Lang) -> Option<String> {
    let mut state = Ask {
        lang,
        // SAFETY: no arguments.
        sequence: unsafe { GetClipboardSequenceNumber() },
        content: HSTRING::new(),
        code: None,
    };
    let content = state.prompt();
    let buttons = [(ID_ENROLL, Text::Enroll)];
    show(Dialog {
        lang,
        icon: PCWSTR::null(),
        content: &content,
        buttons: &buttons,
        common: TDCBF_CANCEL_BUTTON,
        flags: TDF_CALLBACK_TIMER,
        callback: Some(ask_callback),
        data: (&raw mut state) as isize,
    });
    state.code
}

struct Ask {
    lang: Lang,
    sequence: u32,
    /// The text shown; kept alive while the dialog shows it.
    content: HSTRING,
    code: Option<String>,
}

impl Ask {
    fn prompt(&self) -> String {
        match offered_code(clipboard_code()) {
            Some((_, address)) => {
                texts::t(self.lang, Text::ClipboardReady).replace("{address}", &plain(&address))
            }
            None => texts::t(self.lang, Text::CopyThenEnroll).to_owned(),
        }
    }

    fn set_content(&mut self, hwnd: HWND, text: &str) {
        self.content = HSTRING::from(text);
        // SAFETY: the text lives in `self` until the next change or the end of the dialog.
        unsafe {
            SendMessageW(
                hwnd,
                TDM_SET_ELEMENT_TEXT.0 as u32,
                Some(WPARAM(TDE_CONTENT.0 as usize)),
                Some(LPARAM(self.content.as_ptr() as isize)),
            );
        }
    }
}

unsafe extern "system" fn ask_callback(
    hwnd: HWND,
    msg: TASKDIALOG_NOTIFICATIONS,
    wparam: WPARAM,
    _lparam: LPARAM,
    data: isize,
) -> HRESULT {
    // SAFETY: `data` is the `Ask` of `ask`, which outlives the dialog; the dialog calls back on this thread only.
    let state = unsafe { &mut *(data as *mut Ask) };
    match msg {
        TDN_CREATED => {
            // SAFETY: the dialog's window.
            let _ = unsafe { SetForegroundWindow(hwnd) };
        }
        TDN_TIMER => {
            // Follows the clipboard: copying the code in opensecmon changes the text at once.
            // SAFETY: no arguments.
            let sequence = unsafe { GetClipboardSequenceNumber() };
            if sequence != state.sequence {
                state.sequence = sequence;
                let text = state.prompt();
                state.set_content(hwnd, &text);
            }
        }
        TDN_BUTTON_CLICKED if wparam.0 as i32 == ID_ENROLL => match clipboard_code() {
            Some(code) => state.code = Some(code),
            None => {
                let text = format!(
                    "{}\n\n{}",
                    texts::t(state.lang, Text::NoCode),
                    texts::t(state.lang, Text::CopyThenEnroll)
                );
                state.set_content(hwnd, &text);
                return S_FALSE;
            }
        },
        _ => {}
    }
    S_OK
}

/// Enrolls in a thread while a dialog with a moving bar shows.
fn enroll(lang: Lang, store: &Store, default_dir: bool, code: String, address: &str) -> Outcome {
    let done: Arc<Mutex<Option<Outcome>>> = Arc::default();
    let result = Arc::clone(&done);
    let store = store.clone();
    let worker = std::thread::spawn(move || {
        let outcome = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ("other".to_owned(), e.to_string()))
            .and_then(|rt| {
                rt.block_on(crate::enroll_quietly(
                    &code,
                    &Agent::new(store),
                    default_dir,
                ))
                .map_err(|e| (e.kind().to_owned(), e.to_string()))
            });
        *result.lock().unwrap_or_else(|e| e.into_inner()) = Some(outcome);
    });
    let mut state = Wait { done };
    let content = texts::t(lang, Text::Working).replace("{address}", address);
    show(Dialog {
        lang,
        icon: PCWSTR::null(),
        content: &content,
        buttons: &[],
        common: TDCBF_CANCEL_BUTTON,
        flags: TDF_CALLBACK_TIMER | TDF_SHOW_MARQUEE_PROGRESS_BAR,
        callback: Some(wait_callback),
        data: (&raw mut state) as isize,
    });
    // The dialog closes only when the thread is done (or could not be shown at all).
    let _ = worker.join();
    let outcome = state.done.lock().unwrap_or_else(|e| e.into_inner()).take();
    outcome.unwrap_or_else(|| Err(("other".to_owned(), "the enrollment stopped".to_owned())))
}

struct Wait {
    done: Arc<Mutex<Option<Outcome>>>,
}

impl Wait {
    fn is_done(&self) -> bool {
        self.done
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

unsafe extern "system" fn wait_callback(
    hwnd: HWND,
    msg: TASKDIALOG_NOTIFICATIONS,
    _wparam: WPARAM,
    _lparam: LPARAM,
    data: isize,
) -> HRESULT {
    // SAFETY: `data` is the `Wait` of `enroll`, which outlives the dialog.
    let state = unsafe { &*(data as *const Wait) };
    let send = |msg: u32, wparam: usize, lparam: isize| {
        // SAFETY: messages of the task dialog to its own window.
        unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))) };
    };
    match msg {
        TDN_CREATED => {
            send(TDM_SET_PROGRESS_BAR_MARQUEE.0 as u32, 1, 0);
            // There is nothing to cancel once the code is on its way.
            send(TDM_ENABLE_BUTTON.0 as u32, IDCANCEL as usize, 0);
            // SAFETY: the dialog's window.
            let _ = unsafe { SetForegroundWindow(hwnd) };
        }
        TDN_TIMER if state.is_done() => {
            send(TDM_ENABLE_BUTTON.0 as u32, IDCANCEL as usize, 1);
            send(TDM_CLICK_BUTTON.0 as u32, IDCANCEL as usize, 0);
        }
        // Closing (Esc, the window's X) waits for the enrollment.
        TDN_BUTTON_CLICKED if !state.is_done() => return S_FALSE,
        _ => {}
    }
    S_OK
}

/// A message with an OK button.
fn message(lang: Lang, icon: PCWSTR, text: &str) {
    show(Dialog {
        lang,
        icon,
        content: text,
        buttons: &[],
        common: TDCBF_CLOSE_BUTTON,
        flags: TASKDIALOG_FLAGS(0),
        callback: None,
        data: 0,
    });
}

/// An error with "Try again" and "Cancel"; whether to try again.
fn try_again(lang: Lang, text: &str) -> bool {
    show(Dialog {
        lang,
        icon: TD_ERROR_ICON,
        content: text,
        buttons: &[(ID_TRY_AGAIN, Text::TryAgain)],
        common: TDCBF_CANCEL_BUTTON,
        flags: TASKDIALOG_FLAGS(0),
        callback: None,
        data: 0,
    }) == ID_TRY_AGAIN
}

struct Dialog<'a> {
    lang: Lang,
    icon: PCWSTR,
    content: &'a str,
    buttons: &'a [(i32, Text)],
    common: TASKDIALOG_COMMON_BUTTON_FLAGS,
    flags: TASKDIALOG_FLAGS,
    callback: Option<
        unsafe extern "system" fn(HWND, TASKDIALOG_NOTIFICATIONS, WPARAM, LPARAM, isize) -> HRESULT,
    >,
    data: isize,
}

/// Shows a task dialog; the id of the button that closed it (Cancel if it could not be shown).
fn show(d: Dialog<'_>) -> i32 {
    let title = HSTRING::from(texts::t(d.lang, Text::Title));
    let content = HSTRING::from(d.content);
    let labels: Vec<HSTRING> = d
        .buttons
        .iter()
        .map(|&(_, text)| HSTRING::from(texts::t(d.lang, text)))
        .collect();
    let buttons: Vec<TASKDIALOG_BUTTON> = d
        .buttons
        .iter()
        .zip(&labels)
        .map(|(&(id, _), label)| TASKDIALOG_BUTTON {
            nButtonID: id,
            pszButtonText: PCWSTR(label.as_ptr()),
        })
        .collect();
    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        dwFlags: d.flags,
        dwCommonButtons: d.common,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        Anonymous1: TASKDIALOGCONFIG_0 {
            pszMainIcon: d.icon,
        },
        pszMainInstruction: PCWSTR(title.as_ptr()),
        pszContent: PCWSTR(content.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: if buttons.is_empty() {
            std::ptr::null()
        } else {
            buttons.as_ptr()
        },
        nDefaultButton: buttons.first().map_or(0, |b| b.nButtonID),
        pfCallback: d.callback,
        lpCallbackData: d.data,
        ..Default::default()
    };
    let mut pressed = IDCANCEL;
    // SAFETY: all strings and buttons live until the dialog is closed; the callback data too (see the callers).
    if unsafe { TaskDialogIndirect(&config, Some(&mut pressed), None, None) }.is_err() {
        return IDCANCEL;
    }
    pressed
}

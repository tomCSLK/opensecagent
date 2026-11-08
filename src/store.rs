//! What the agent keeps on the computer, all in one folder that only it (root or SYSTEM) can read:
//!
//! - `enrollment.json`: address of opensecmon, fingerprint of its certificate and the name of the computer there,
//! - `token`: the token of this computer (secret),
//! - `state.json`: hash and time of the last report, the interval opensecmon asked for, the last error.
//!
//! Linux: `/var/lib/opensecagent` (folder `700`, files `600`). Windows: `%ProgramData%\opensecagent` with an access
//! list only for SYSTEM and the administrators, without inheritance from `ProgramData` (`winsvc::folder`); the files
//! inherit it. Files are replaced through a temporary file in the same folder, so a crash never leaves half a token
//! behind.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::code;

const ENROLLMENT: &str = "enrollment.json";
const TOKEN: &str = "token";
const STATE: &str = "state.json";
/// Larger files are not ours (the token has 48 characters, the others a few hundred bytes).
const FILE_MAX: u64 = 64 * 1024;

/// The folder of the agent's files unless `--dir` or `OPENSECAGENT_DIR` names another one.
pub fn default_dir() -> PathBuf {
    if cfg!(windows) {
        let base = std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into());
        PathBuf::from(base).join("opensecagent")
    } else {
        PathBuf::from("/var/lib/opensecagent")
    }
}

/// How this computer is enrolled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Enrollment {
    /// Address of opensecmon as in the enrollment code, e.g. `192.0.2.10:8444`.
    pub address: String,
    /// Fingerprint of the certificate of the built-in HTTPS (`AB:CD:…`), if the code carried one.
    pub fingerprint: Option<String>,
    /// The name of the computer in opensecmon.
    pub device: String,
    /// When it was enrolled (seconds since 1970).
    pub enrolled_at: u64,
}

impl Enrollment {
    /// The fingerprint as bytes; `None` without one or if the file was changed into something else.
    pub fn pin(&self) -> Option<[u8; 32]> {
        self.fingerprint
            .as_deref()
            .and_then(code::parse_fingerprint)
    }
}

/// The token of this computer. Never printed: `Debug` shows only the prefix.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// A token as opensecmon hands it out (`osm1_` and Base64url); `None` for anything else, so nothing odd ever
    /// lands in a header.
    pub fn new(text: &str) -> Option<Self> {
        let text = text.trim();
        (text.starts_with("osm1_")
            && text.len() <= 128
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
        .then(|| Self(text.to_owned()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(osm1_…)")
    }
}

/// What the agent remembers between runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// SHA-256 (hex) of the last full report opensecmon accepted; with it an unchanged list goes as the short form.
    pub last_hash: Option<String>,
    /// When opensecmon last accepted a report (seconds since 1970).
    pub last_report: Option<u64>,
    /// The interval opensecmon asked for in its last answer.
    pub next_report_s: Option<u64>,
    /// opensecmon answered `invalid_token`: the computer was revoked or deleted, reporting stops until it is
    /// enrolled again.
    #[serde(default)]
    pub revoked: bool,
    /// The last failure in words, for `status`; cleared by the next success.
    pub last_error: Option<String>,
}

/// The agent's folder.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
    /// The agent's own folder (`default_dir`), which on Windows gets its own access list. A folder named with
    /// `--dir` is left as it is, it may be one of the user's.
    #[cfg_attr(not(windows), allow(dead_code))]
    own: bool,
}

impl Store {
    /// A folder named with `--dir` (or in tests).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            own: false,
        }
    }

    /// The agent's own folder.
    pub fn own() -> Self {
        Self {
            dir: default_dir(),
            own: true,
        }
    }

    /// Makes sure the folder exists and that only the agent can get in: on Windows the access list of the own
    /// folder is set again (and a folder someone else made, or a link, is refused); elsewhere only the folder is
    /// created if missing.
    pub fn secure(&self) -> io::Result<()> {
        #[cfg(windows)]
        if self.own {
            return crate::winsvc::folder::protect(&self.dir);
        }
        self.ensure_dir()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The enrollment, `None` if the computer is not enrolled.
    pub fn enrollment(&self) -> io::Result<Option<Enrollment>> {
        let Some(bytes) = self.read(ENROLLMENT)? else {
            return Ok(None);
        };
        let e: Enrollment = serde_json::from_slice(&bytes).map_err(|e| invalid(ENROLLMENT, e))?;
        if !code::is_address(&e.address) {
            return Err(invalid(ENROLLMENT, "the address is not valid"));
        }
        if e.fingerprint.is_some() && e.pin().is_none() {
            return Err(invalid(ENROLLMENT, "the fingerprint is not valid"));
        }
        Ok(Some(e))
    }

    /// The token, `None` if there is none.
    pub fn token(&self) -> io::Result<Option<Token>> {
        let Some(bytes) = self.read(TOKEN)? else {
            return Ok(None);
        };
        let text = String::from_utf8(bytes).map_err(|_| invalid(TOKEN, "not text"))?;
        Token::new(&text)
            .map(Some)
            .ok_or_else(|| invalid(TOKEN, "not a token of opensecmon"))
    }

    /// The state; a missing or unreadable file counts as a fresh start (the next report is then a full one).
    pub fn state(&self) -> State {
        self.read(STATE)
            .ok()
            .flatten()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Keeps a new enrollment: token first, then the enrollment, then a fresh state.
    pub fn save_enrollment(&self, enrollment: &Enrollment, token: &Token) -> io::Result<()> {
        self.secure()?;
        self.write(TOKEN, format!("{}\n", token.expose()).as_bytes())?;
        self.write(ENROLLMENT, &to_json(enrollment))?;
        self.save_state(&State::default())
    }

    pub fn save_state(&self, state: &State) -> io::Result<()> {
        self.ensure_dir()?;
        self.write(STATE, &to_json(state))
    }

    /// Removes the agent's files (not the folder, it may be one the user named); whether there were any.
    pub fn forget(&self) -> io::Result<bool> {
        let mut any = false;
        for name in [TOKEN, ENROLLMENT, STATE] {
            match fs::remove_file(self.dir.join(name)) {
                Ok(()) => any = true,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(any)
    }

    fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        let path = self.dir.join(name);
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if file.metadata()?.len() > FILE_MAX {
            return Err(invalid(name, "too large"));
        }
        let mut bytes = Vec::new();
        io::Read::read_to_end(&mut io::Read::take(file, FILE_MAX), &mut bytes)?;
        Ok(Some(bytes))
    }

    /// Creates the folder, only accessible to the owner. An existing folder is left as it is.
    fn ensure_dir(&self) -> io::Result<()> {
        if self.dir.is_dir() {
            return Ok(());
        }
        #[cfg(windows)]
        if self.own {
            return crate::winsvc::folder::protect(&self.dir);
        }
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&self.dir)
    }

    /// Writes a file readable only by the owner, through a temporary file and a rename.
    fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.dir.join(name);
        let temp = self.dir.join(format!(".{name}.new"));
        let _ = fs::remove_file(&temp);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let result = (|| {
            let mut file = options.open(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, &path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}

fn to_json<T: Serialize>(value: &T) -> Vec<u8> {
    let mut out = serde_json::to_vec_pretty(value).expect("plain data");
    out.push(b'\n');
    out
}

fn invalid(name: &str, why: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{name}: {why}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN_TEXT: &str = "osm1_3q2-7wSxB0hLk8f_u7Yc1NJdQe9vX4aZpR6mT2cKgWo";

    fn enrollment() -> Enrollment {
        Enrollment {
            address: "192.0.2.10:8444".into(),
            fingerprint: Some(code::format_fingerprint(&[9; 32])),
            device: "laptop".into(),
            enrolled_at: 1_762_500_000,
        }
    }

    #[test]
    fn an_enrollment_is_kept_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("agent"));
        assert_eq!(store.enrollment().unwrap(), None);
        assert_eq!(store.token().unwrap(), None);
        assert_eq!(store.state(), State::default());

        let token = Token::new(TOKEN_TEXT).unwrap();
        store.save_enrollment(&enrollment(), &token).unwrap();
        assert_eq!(store.enrollment().unwrap(), Some(enrollment()));
        assert_eq!(store.enrollment().unwrap().unwrap().pin(), Some([9; 32]));
        assert_eq!(store.token().unwrap(), Some(token));

        let state = State {
            last_hash: Some("ab".repeat(32)),
            last_report: Some(5),
            next_report_s: Some(900),
            revoked: false,
            last_error: None,
        };
        store.save_state(&state).unwrap();
        assert_eq!(store.state(), state);

        assert!(store.forget().unwrap());
        assert!(!store.forget().unwrap());
        assert_eq!(store.enrollment().unwrap(), None);
        assert!(store.dir().is_dir(), "the folder stays");
    }

    #[cfg(unix)]
    #[test]
    fn only_the_owner_can_read_the_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("agent"));
        store
            .save_enrollment(&enrollment(), &Token::new(TOKEN_TEXT).unwrap())
            .unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(store.dir()), 0o700);
        for name in [TOKEN, ENROLLMENT, STATE] {
            assert_eq!(mode(&store.dir().join(name)), 0o600, "{name}");
        }
        // No temporary files are left.
        let names: Vec<_> = fs::read_dir(store.dir()).unwrap().collect();
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn the_token_never_shows_in_debug_output() {
        let token = Token::new(TOKEN_TEXT).unwrap();
        assert!(!format!("{token:?}").contains("3q2"));
        assert_eq!(token.expose(), TOKEN_TEXT);
        assert_eq!(
            Token::new(&format!(" {TOKEN_TEXT}\n")).unwrap().expose(),
            TOKEN_TEXT
        );
        for bad in [
            "",
            "osm2_abc",
            "osm1_a b",
            "osm1_abc\r\nX-Evil: 1",
            &format!("osm1_{}", "a".repeat(200)),
        ] {
            assert_eq!(Token::new(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn changed_or_damaged_files_are_errors_or_a_fresh_start() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        fs::write(dir.path().join(TOKEN), "not a token").unwrap();
        assert!(store.token().is_err());
        fs::write(dir.path().join(ENROLLMENT), "{").unwrap();
        assert!(store.enrollment().is_err());
        let mut bad = enrollment();
        bad.address = "host/path".into();
        fs::write(dir.path().join(ENROLLMENT), to_json(&bad)).unwrap();
        assert!(store.enrollment().is_err());
        let mut bad = enrollment();
        bad.fingerprint = Some("AB:CD".into());
        fs::write(dir.path().join(ENROLLMENT), to_json(&bad)).unwrap();
        assert!(store.enrollment().is_err());
        fs::write(dir.path().join(STATE), "[]").unwrap();
        assert_eq!(store.state(), State::default());
        fs::write(dir.path().join(STATE), vec![b' '; 70_000]).unwrap();
        assert_eq!(store.state(), State::default());
    }
}

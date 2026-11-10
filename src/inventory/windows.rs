//! The inventory of Windows: what the script `opensecmon-agent.ps1` reads, plus the Pythons of the Python install
//! manager.
//!
//! - `os`: the version of Windows from `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`.
//! - `win`: the Uninstall entries of the machine (64 and 32 bit) and of every user, without system components, parts
//!   of other entries and updates, as the script. Users signed in from their hives under `HKEY_USERS`, the others
//!   from their `NTUSER.DAT`, read as a file and never loaded (`hive.rs`, PLAN.md step 3b). The user is never named.
//! - `win` as well: Pythons registered under `Software\Python\PythonCore` (PEP 514) that have no Uninstall entry
//!   of their own, as the Python install manager installs them.
//! - `appx`: the store apps of all users, without frameworks and resource packages.
//!
//! Values of the registry and of the store apps are data: they are only cut to the limits of the API.

use std::collections::{BTreeMap, BTreeSet};

use super::hive::{self, Hive, Snapshot};
use super::{Inventory, InventoryError, Item, Os, field};

/// opensecmon takes at most this many entries per report (docs/agent-api.md there).
const MAX_ITEMS: usize = 50_000;
const CURRENT_VERSION: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const UNINSTALL: &str = r"Microsoft\Windows\CurrentVersion\Uninstall";
const PROFILE_LIST: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList";
/// What the inventory reads from the hive of a user (`programs`, `pythons`).
const USER_PLACES: [&str; 2] = [
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
    r"Software\Python\PythonCore",
];

pub fn collect() -> Result<Inventory, InventoryError> {
    let mut notes = Vec::new();
    let offline = offline_users(&SystemRegistry, &read_hive_file, &mut notes);
    let registry = WithOffline {
        live: &SystemRegistry,
        offline,
    };
    let os = os_info(
        &registry,
        &std::env::var("PROCESSOR_ARCHITECTURE").unwrap_or_default(),
    )?;
    let mut items = programs(&registry, &mut notes);
    let pythons = pythons(&registry, &items, &mut notes);
    items.extend(pythons);
    items.extend(store_apps(&mut notes)?);
    for note in notes {
        // standard error (`inventory` writes the report on standard output), or the log of the service
        crate::log::note(format_args!("note: {note}"));
    }
    let mut items = super::sorted(items);
    items.truncate(MAX_ITEMS);
    Ok(Inventory {
        hostname: super::hostname(),
        os,
        dpkg: None,
        items,
    })
}

/// A value of the registry, as far as the inventory needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Text(String),
    Number(u64),
}

impl Value {
    fn text(&self) -> String {
        match self {
            Value::Text(s) => s.clone(),
            Value::Number(n) => n.to_string(),
        }
    }

    fn number(&self) -> Option<u64> {
        match self {
            Value::Text(s) => s.trim().parse().ok(),
            Value::Number(n) => Some(*n),
        }
    }
}

/// Where the inventory reads the registry: the real one or, in the tests, a stand-in. Paths start with `HKLM\` or
/// `HKU\`.
trait Registry {
    /// The names of the subkeys; `Ok(None)` if the key does not exist.
    fn subkeys(&self, path: &str) -> Result<Option<Vec<String>>, String>;
    /// A value of a key; `None` if the key or the value does not exist or has a type the inventory does not read.
    fn value(&self, path: &str, name: &str) -> Option<Value>;
}

fn text(registry: &dyn Registry, path: &str, name: &str, max: usize) -> String {
    registry
        .value(path, name)
        .map(|v| field(&v.text(), max))
        .unwrap_or_default()
}

/// The system, as `os` of the report. `CurrentBuildNumber` and `UBR` are required by opensecmon.
fn os_info(registry: &dyn Registry, processor: &str) -> Result<Os, InventoryError> {
    let err =
        |what: &str| InventoryError::Read("the version of Windows", format!("{what} is missing"));
    let number = |name: &str| {
        registry
            .value(CURRENT_VERSION, name)
            .and_then(|v| v.number())
            .and_then(|n| u32::try_from(n).ok())
    };
    let build = number("CurrentBuildNumber").ok_or_else(|| err("CurrentBuildNumber"))?;
    let ubr = number("UBR").ok_or_else(|| err("UBR"))?;
    let mut display = text(registry, CURRENT_VERSION, "DisplayVersion", 20);
    if display.is_empty() {
        display = text(registry, CURRENT_VERSION, "ReleaseId", 20);
    }
    let some = |s: String| (!s.is_empty()).then_some(s);
    Ok(Os {
        family: "windows",
        product_name: some(text(registry, CURRENT_VERSION, "ProductName", 200)),
        display_version: some(display),
        build: Some(build),
        ubr: Some(ubr),
        installation_type: some(text(registry, CURRENT_VERSION, "InstallationType", 40)),
        arch: arch(processor).to_owned(),
        ..Os::default()
    })
}

/// The architecture as the scripts name it, from `PROCESSOR_ARCHITECTURE`.
fn arch(processor: &str) -> &'static str {
    match processor.to_ascii_uppercase().as_str() {
        "AMD64" => "x64",
        "ARM64" => "arm64",
        "X86" => "x86",
        _ => "other",
    }
}

/// The hives of people signed in: `HKEY_USERS\S-1-5-21-…` (not the `_Classes` keys, not the service accounts).
fn user_hives(registry: &dyn Registry, notes: &mut Vec<String>) -> Vec<String> {
    match registry.subkeys("HKU") {
        Ok(keys) => {
            let mut sids: Vec<String> = keys
                .unwrap_or_default()
                .into_iter()
                .filter(|k| is_user_sid(k))
                .collect();
            sids.sort();
            sids
        }
        Err(e) => {
            notes.push(format!("the users signed in could not be listed: {e}"));
            Vec::new()
        }
    }
}

fn is_user_sid(key: &str) -> bool {
    key.strip_prefix("S-1-5-21-").is_some_and(|rest| {
        !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit() || b == b'-')
    })
}

/// The Uninstall entries of the machine and of the users signed in, each key name once (the machine first).
fn programs(registry: &dyn Registry, notes: &mut Vec<String>) -> Vec<Item> {
    let mut places = vec![
        (format!(r"HKLM\SOFTWARE\{UNINSTALL}"), "machine"),
        (format!(r"HKLM\SOFTWARE\WOW6432Node\{UNINSTALL}"), "machine"),
    ];
    for sid in user_hives(registry, notes) {
        places.push((format!(r"HKU\{sid}\Software\{UNINSTALL}"), "user"));
    }
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    for (place, scope) in places {
        let keys = match registry.subkeys(&place) {
            Ok(keys) => keys.unwrap_or_default(),
            Err(e) => {
                // The user is not named, not even by the SID.
                let whose = if scope == "user" {
                    "of a user"
                } else {
                    "of the machine"
                };
                notes.push(format!("the programs {whose} could not be read: {e}"));
                continue;
            }
        };
        for key in keys {
            let path = format!(r"{place}\{key}");
            let Some(item) = program(registry, &path, &key, scope) else {
                continue;
            };
            if seen.insert(item.id.clone()) {
                items.push(item);
            }
        }
    }
    items
}

/// One Uninstall entry, or `None` for what the script leaves out as well.
fn program(registry: &dyn Registry, path: &str, key: &str, scope: &'static str) -> Option<Item> {
    let name = text(registry, path, "DisplayName", 200);
    if name.is_empty() {
        return None;
    }
    if registry
        .value(path, "SystemComponent")
        .and_then(|v| v.number())
        == Some(1)
    {
        return None;
    }
    if !text(registry, path, "ParentKeyName", 200).is_empty() {
        return None;
    }
    let release_type = text(registry, path, "ReleaseType", 40);
    if ["Update", "Hotfix", "Security Update"].contains(&release_type.as_str()) {
        return None;
    }
    let id = field(key, 200);
    if id.is_empty() {
        return None;
    }
    let publisher = text(registry, path, "Publisher", 200);
    Some(Item {
        kind: "win",
        id,
        name,
        version: text(registry, path, "DisplayVersion", 100),
        publisher: (!publisher.is_empty()).then_some(publisher),
        winget_id: None,
        scope: Some(scope),
    })
}

/// Pythons registered by PEP 514 under `Software\Python\PythonCore\<tag>` that no Uninstall entry names, as the
/// Python install manager installs them (the installer of python.org writes an Uninstall entry as well, "Python
/// 3.14.10 (64-bit)"). Sent as `win` entries with the ID `Python\PythonCore\<tag>`, so opensecmon resolves them
/// like the entries of the installer.
fn pythons(registry: &dyn Registry, programs: &[Item], notes: &mut Vec<String>) -> Vec<Item> {
    let mut places = vec![
        (r"HKLM\SOFTWARE\Python".to_owned(), "machine"),
        (r"HKLM\SOFTWARE\WOW6432Node\Python".to_owned(), "machine"),
    ];
    for sid in user_hives(registry, &mut Vec::new()) {
        places.push((format!(r"HKU\{sid}\Software\Python"), "user"));
    }
    let installed: Vec<String> = programs.iter().map(|p| p.name.to_lowercase()).collect();
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    for (place, scope) in places {
        let company = format!(r"{place}\PythonCore");
        let tags = match registry.subkeys(&company) {
            Ok(tags) => tags.unwrap_or_default(),
            Err(e) => {
                let whose = if scope == "user" {
                    "of a user"
                } else {
                    "of the machine"
                };
                notes.push(format!("the Pythons {whose} could not be read: {e}"));
                continue;
            }
        };
        let publisher = text(registry, &company, "DisplayName", 200);
        for tag in tags {
            let path = format!(r"{company}\{tag}");
            // a key without a version is left over from an uninstalled Python
            let version = text(registry, &path, "Version", 100);
            if version.is_empty() {
                continue;
            }
            let prefix = format!("python {} ", version.to_lowercase());
            if installed
                .iter()
                .any(|n| n.starts_with(&prefix) || *n == prefix.trim_end())
            {
                continue;
            }
            let id = field(&format!(r"Python\PythonCore\{tag}"), 200);
            if !seen.insert(id.clone()) {
                continue;
            }
            let mut name = text(registry, &path, "DisplayName", 200);
            if name.is_empty() {
                name = field(&format!("Python {tag}"), 200);
            }
            items.push(Item {
                kind: "win",
                id,
                name,
                version,
                publisher: Some(if publisher.is_empty() {
                    "Python Software Foundation".to_owned()
                } else {
                    publisher.clone()
                }),
                winget_id: None,
                scope: Some(scope),
            });
        }
    }
    items
}

/// The hives of users not signed in, from their `NTUSER.DAT` (profiles from `ProfileList`), keyed by SID. Only
/// [`USER_PLACES`] are kept; each file is read, copied out and dropped before the next. A file that cannot be read is
/// left out with a note that names no one (the profile folder carries the user's name, so neither the path nor the
/// SID is written); a profile without the file (folder deleted) is left out silently.
fn offline_users(
    live: &dyn Registry,
    read_file: &dyn Fn(&str) -> std::io::Result<Vec<u8>>,
    notes: &mut Vec<String>,
) -> BTreeMap<String, Snapshot> {
    let signed_in: BTreeSet<String> = user_hives(live, &mut Vec::new())
        .iter()
        .map(|s| s.to_ascii_uppercase())
        .collect();
    let profiles = match live.subkeys(PROFILE_LIST) {
        Ok(keys) => keys.unwrap_or_default(),
        Err(e) => {
            notes.push(format!("the user profiles could not be listed: {e}"));
            return BTreeMap::new();
        }
    };
    let mut out = BTreeMap::new();
    for sid in profiles.into_iter().filter(|k| is_user_sid(k)) {
        if signed_in.contains(&sid.to_ascii_uppercase()) {
            continue;
        }
        let dir = live
            .value(&format!(r"{PROFILE_LIST}\{sid}"), "ProfileImagePath")
            .map(|v| v.text())
            .unwrap_or_default();
        if dir.trim().is_empty() {
            continue;
        }
        let dir = expand(dir.trim(), |name| std::env::var(name).ok());
        let file = format!(r"{}\NTUSER.DAT", dir.trim_end_matches('\\'));
        let bytes = match read_file(&file) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                notes.push(format!(
                    "the programs of a user not signed in could not be read: {e}"
                ));
                continue;
            }
        };
        match Hive::parse(bytes).and_then(|h| Snapshot::take(&h, &USER_PLACES)) {
            Ok(snapshot) => {
                out.insert(sid, snapshot);
            }
            Err(e) => notes.push(format!(
                "the programs of a user not signed in could not be read: {e}"
            )),
        }
    }
    out
}

/// `%NAME%` replaced by the variable of the environment, as Windows expands `REG_EXPAND_SZ` (`ProfileImagePath` is
/// usually `C:\Users\…`, sometimes `%SystemDrive%\Users\…`). Unknown names stay as they are.
fn expand(text: &str, var: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        match (!name.is_empty()).then(|| var(name)).flatten() {
            Some(value) => out.push_str(&value),
            None => {
                out.push('%');
                out.push_str(name);
                out.push('%');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Reads a hive file of a user not signed in. The file is opened only for reading, with every share mode (Windows
/// and other programs may open it meanwhile) and only for as long as reading takes; a link at its place (the user
/// owns the folder) is not followed but refused, as is a file larger than [`hive::MAX_FILE`].
fn read_hive_file(path: &str) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

    use ::windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_SEQUENTIAL_SCAN,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN).0)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(std::io::Error::other("not a plain file"));
    }
    let too_large = || std::io::Error::other(hive::HiveError::TooLarge.to_string());
    if meta.len() > hive::MAX_FILE {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.by_ref()
        .take(hive::MAX_FILE + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > hive::MAX_FILE {
        return Err(too_large());
    }
    Ok(bytes)
}

/// The registry of this computer with the users not signed in under `HKU\<SID>`, as if their hives were loaded.
struct WithOffline<'a> {
    live: &'a dyn Registry,
    offline: BTreeMap<String, Snapshot>,
}

impl WithOffline<'_> {
    /// The copied hive and the path inside it, for a path under `HKU\<SID>` of a user not signed in.
    fn offline<'p>(&self, path: &'p str) -> Option<(&Snapshot, &'p str)> {
        let rest = path.strip_prefix(r"HKU\")?;
        let (sid, rest) = rest.split_once('\\').unwrap_or((rest, ""));
        self.offline.get(sid).map(|s| (s, rest))
    }
}

impl Registry for WithOffline<'_> {
    fn subkeys(&self, path: &str) -> Result<Option<Vec<String>>, String> {
        if path == "HKU" {
            let mut keys = self.live.subkeys(path)?.unwrap_or_default();
            for sid in self.offline.keys() {
                if !keys.iter().any(|k| k.eq_ignore_ascii_case(sid)) {
                    keys.push(sid.clone());
                }
            }
            return Ok(Some(keys));
        }
        match self.offline(path) {
            Some((snapshot, rest)) => Ok(snapshot.subkeys(rest)),
            None => self.live.subkeys(path),
        }
    }

    fn value(&self, path: &str, name: &str) -> Option<Value> {
        match self.offline(path) {
            Some((snapshot, rest)) => match snapshot.value(rest, name)? {
                hive::Data::Text(s) => Some(Value::Text(s)),
                hive::Data::Number(n) => Some(Value::Number(n)),
            },
            None => self.live.value(path, name),
        }
    }
}

/// A store app as `appx` of the report; `None` without a package family name.
fn store_item(family: &str, name: &str, version: &str, publisher: &str) -> Option<Item> {
    let id = field(family, 200);
    if id.is_empty() {
        return None;
    }
    let publisher = field(&common_name(publisher), 200);
    Some(Item {
        kind: "appx",
        id,
        name: field(name, 200),
        version: field(version, 100),
        publisher: (!publisher.is_empty()).then_some(publisher),
        winget_id: None,
        // installed per user, as in the script
        scope: Some("user"),
    })
}

/// The name of the publisher from its distinguished name: "CN=Microsoft Corporation, O=…, C=US" → "Microsoft
/// Corporation", with a comma in quotes: `CN="Anthropic, PBC", O=…` → "Anthropic, PBC" (the script cut that to
/// `"Anthropic`). Anything else stays as it is.
fn common_name(dn: &str) -> String {
    let Some(rest) = dn.trim().strip_prefix("CN=") else {
        return dn.to_owned();
    };
    if let Some(quoted) = rest.strip_prefix('"') {
        // `""` stands for a quote inside the value
        let mut out = String::new();
        let mut chars = quoted.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                } else {
                    return out;
                }
            }
            out.push(c);
        }
        return out;
    }
    rest.split(',').next().unwrap_or_default().to_owned()
}

/// The store apps of all users, once per package family. Without administrator rights (trying the agent by hand)
/// only those of this user, with a note; the service runs as SYSTEM and sees all.
fn store_apps(notes: &mut Vec<String>) -> Result<Vec<Item>, InventoryError> {
    use ::windows::Management::Deployment::PackageManager;
    use ::windows::core::HSTRING;

    let err = |e: ::windows::core::Error| InventoryError::Read("the store apps", e.message());
    let manager = PackageManager::new().map_err(err)?;
    let packages = match manager.FindPackages() {
        Ok(packages) => packages,
        Err(e) if e.code().0 == ACCESS_DENIED => {
            notes.push(
                "without administrator rights only the store apps of this user are read".into(),
            );
            manager
                .FindPackagesByUserSecurityId(&HSTRING::new())
                .map_err(err)?
        }
        Err(e) => return Err(err(e)),
    };
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    let it = packages.First().map_err(err)?;
    while it.HasCurrent().map_err(err)? {
        if let Ok(package) = it.Current() {
            let wanted = !package.IsFramework().unwrap_or(true)
                && !package.IsResourcePackage().unwrap_or(true);
            if let (true, Ok(id)) = (wanted, package.Id()) {
                let family = id.FamilyName().map(|s| s.to_string()).unwrap_or_default();
                let v = id.Version().unwrap_or_default();
                let version = format!("{}.{}.{}.{}", v.Major, v.Minor, v.Build, v.Revision);
                let item = store_item(
                    &family,
                    &id.Name().map(|s| s.to_string()).unwrap_or_default(),
                    &version,
                    &id.Publisher().map(|s| s.to_string()).unwrap_or_default(),
                );
                if let Some(item) = item.filter(|i| seen.insert(i.id.clone())) {
                    items.push(item);
                }
            }
        }
        it.MoveNext().map_err(err)?;
    }
    Ok(items)
}

/// The registry of this computer, read only (the 64-bit view, as the agent is a 64-bit program).
struct SystemRegistry;

impl SystemRegistry {
    fn open(path: &str) -> windows_registry::Result<windows_registry::Key> {
        let (root, rest) = path.split_once('\\').unwrap_or((path, ""));
        let root = match root {
            "HKLM" => windows_registry::LOCAL_MACHINE,
            _ => windows_registry::USERS,
        };
        root.options().read().open(rest)
    }
}

/// `ERROR_FILE_NOT_FOUND` as HRESULT: the key does not exist.
const NOT_FOUND: i32 = 0x8007_0002_u32 as i32;
/// `E_ACCESSDENIED`.
const ACCESS_DENIED: i32 = 0x8007_0005_u32 as i32;

impl Registry for SystemRegistry {
    fn subkeys(&self, path: &str) -> Result<Option<Vec<String>>, String> {
        let key = match Self::open(path) {
            Ok(key) => key,
            Err(e) if e.code().0 == NOT_FOUND => return Ok(None),
            Err(e) => return Err(e.message()),
        };
        let keys = key.keys().map_err(|e| e.message())?;
        Ok(Some(keys.collect()))
    }

    fn value(&self, path: &str, name: &str) -> Option<Value> {
        use windows_registry::Type;
        let value = Self::open(path).ok()?.get_value(name).ok()?;
        match value.ty() {
            Type::String | Type::ExpandString => String::try_from(value).ok().map(Value::Text),
            Type::U32 | Type::U64 => u64::try_from(value).ok().map(Value::Number),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// A registry in memory: keys with their values; subkeys are taken from the paths.
    #[derive(Default)]
    struct FakeRegistry {
        keys: BTreeMap<String, BTreeMap<String, Value>>,
        denied: BTreeSet<String>,
    }

    impl FakeRegistry {
        fn key(&mut self, path: &str, values: &[(&str, Value)]) -> &mut Self {
            let entry = self.keys.entry(path.to_owned()).or_default();
            for (name, value) in values {
                entry.insert((*name).to_owned(), value.clone());
            }
            self
        }

        fn deny(&mut self, path: &str) -> &mut Self {
            self.denied.insert(path.to_owned());
            self
        }
    }

    impl Registry for FakeRegistry {
        fn subkeys(&self, path: &str) -> Result<Option<Vec<String>>, String> {
            if self.denied.contains(path) {
                return Err("Access is denied.".into());
            }
            let prefix = format!(r"{path}\");
            let mut found = false;
            let mut names = BTreeSet::new();
            for key in self.keys.keys() {
                if key == path {
                    found = true;
                } else if let Some(rest) = key.strip_prefix(&prefix) {
                    found = true;
                    names.insert(rest.split('\\').next().unwrap_or_default().to_owned());
                }
            }
            Ok(found.then(|| names.into_iter().collect()))
        }

        fn value(&self, path: &str, name: &str) -> Option<Value> {
            self.keys.get(path)?.get(name).cloned()
        }
    }

    fn t(s: &str) -> Value {
        Value::Text(s.to_owned())
    }

    const MACHINE: &str = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    const MACHINE32: &str = r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
    const ALICE: &str = r"HKU\S-1-5-21-1111-2222-3333-1001";

    fn windows_11() -> FakeRegistry {
        let mut r = FakeRegistry::default();
        r.key(
            CURRENT_VERSION,
            &[
                ("ProductName", t("Windows 10 Pro")),
                ("DisplayVersion", t("25H2")),
                ("CurrentBuildNumber", t("26200")),
                ("UBR", Value::Number(7171)),
                ("InstallationType", t("Client")),
            ],
        );
        r
    }

    #[test]
    fn the_version_of_windows_is_read_as_the_script_reads_it() {
        let os = os_info(&windows_11(), "AMD64").unwrap();
        assert_eq!(
            serde_json::to_value(os).unwrap(),
            serde_json::json!({ "family": "windows", "product_name": "Windows 10 Pro",
                "display_version": "25H2", "build": 26200, "ubr": 7171, "installation_type": "Client",
                "arch": "x64" })
        );
        // Windows 10 before 20H2 has only ReleaseId
        let mut r = FakeRegistry::default();
        r.key(
            CURRENT_VERSION,
            &[
                ("ReleaseId", t("1809")),
                ("CurrentBuildNumber", t("17763")),
                ("UBR", Value::Number(1)),
            ],
        );
        let os = os_info(&r, "ARM64").unwrap();
        assert_eq!(os.display_version.as_deref(), Some("1809"));
        assert_eq!(os.product_name, None);
        assert_eq!(os.arch, "arm64");
    }

    #[test]
    fn build_and_ubr_are_required() {
        let mut r = FakeRegistry::default();
        r.key(CURRENT_VERSION, &[("CurrentBuildNumber", t("26200"))]);
        assert!(
            os_info(&r, "AMD64")
                .unwrap_err()
                .to_string()
                .contains("UBR")
        );
        let mut r = FakeRegistry::default();
        r.key(
            CURRENT_VERSION,
            &[("CurrentBuildNumber", t("x")), ("UBR", Value::Number(1))],
        );
        assert!(os_info(&r, "AMD64").is_err());
        assert!(os_info(&FakeRegistry::default(), "AMD64").is_err());
    }

    #[test]
    fn architectures_are_named_as_the_api_names_them() {
        assert_eq!(arch("AMD64"), "x64");
        assert_eq!(arch("arm64"), "arm64");
        assert_eq!(arch("x86"), "x86");
        assert_eq!(arch(""), "other");
    }

    #[test]
    fn only_people_count_as_users() {
        assert!(is_user_sid("S-1-5-21-1111-2222-3333-1001"));
        assert!(!is_user_sid("S-1-5-21-1111-2222-3333-1001_Classes"));
        assert!(!is_user_sid("S-1-5-18"));
        assert!(!is_user_sid(".DEFAULT"));
        assert!(!is_user_sid("S-1-5-21-"));
    }

    #[test]
    fn programs_are_filtered_as_in_the_script() {
        let mut r = windows_11();
        r.key(
            &format!(r"{MACHINE}\{{1F1C2DFC}}"),
            &[
                ("DisplayName", t("Mozilla Firefox (x64 de)")),
                ("DisplayVersion", t("144.0.2")),
                ("Publisher", t("Mozilla")),
            ],
        )
        .key(
            &format!(r"{MACHINE}\NoName"),
            &[("DisplayVersion", t("1.0"))],
        )
        .key(
            &format!(r"{MACHINE}\Component"),
            &[
                ("DisplayName", t("Part")),
                ("SystemComponent", Value::Number(1)),
            ],
        )
        .key(
            &format!(r"{MACHINE}\Child"),
            &[("DisplayName", t("Child")), ("ParentKeyName", t("Office"))],
        )
        .key(
            &format!(r"{MACHINE}\KB123"),
            &[
                ("DisplayName", t("Update for X")),
                ("ReleaseType", t("Security Update")),
            ],
        )
        .key(
            &format!(r"{MACHINE32}\7-Zip"),
            &[
                ("DisplayName", t("7-Zip 24.09\n")),
                // some installers write the version as a number
                ("DisplayVersion", Value::Number(2409)),
            ],
        )
        // the same key name for the machine and a user: the machine wins
        .key(
            &format!(r"{MACHINE32}\Shared"),
            &[("DisplayName", t("Shared")), ("DisplayVersion", t("2"))],
        )
        .key(
            &format!(r"{ALICE}\Software\{UNINSTALL}\Shared"),
            &[("DisplayName", t("Shared")), ("DisplayVersion", t("1"))],
        )
        .key(
            &format!(r"{ALICE}\Software\{UNINSTALL}\Zoom"),
            &[
                ("DisplayName", t("Zoom Workplace")),
                ("DisplayVersion", t("6.6.0")),
            ],
        )
        // not a person
        .key(
            &format!(r"HKU\S-1-5-18\Software\{UNINSTALL}\Service"),
            &[("DisplayName", t("Service"))],
        );
        let mut notes = Vec::new();
        let items = programs(&r, &mut notes);
        assert!(notes.is_empty(), "{notes:?}");
        let got: Vec<_> = items
            .iter()
            .map(|i| {
                (
                    i.id.as_str(),
                    i.name.as_str(),
                    i.version.as_str(),
                    i.publisher.as_deref(),
                    i.scope,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (
                    "{1F1C2DFC}",
                    "Mozilla Firefox (x64 de)",
                    "144.0.2",
                    Some("Mozilla"),
                    Some("machine")
                ),
                ("7-Zip", "7-Zip 24.09", "2409", None, Some("machine")),
                ("Shared", "Shared", "2", None, Some("machine")),
                ("Zoom", "Zoom Workplace", "6.6.0", None, Some("user")),
            ]
        );
    }

    #[test]
    fn an_unreadable_hive_is_left_out_with_a_note_that_names_no_one() {
        let mut r = windows_11();
        r.key(&format!(r"{MACHINE}\App"), &[("DisplayName", t("App"))])
            .key(
                &format!(r"{ALICE}\Software\{UNINSTALL}\Other"),
                &[("DisplayName", t("Other"))],
            )
            .deny(&format!(r"{ALICE}\Software\{UNINSTALL}"));
        let mut notes = Vec::new();
        let items = programs(&r, &mut notes);
        assert_eq!(items.len(), 1);
        assert_eq!(notes.len(), 1);
        assert!(
            notes[0].contains("of a user") && !notes[0].contains("S-1-5"),
            "{notes:?}"
        );
    }

    #[test]
    fn pythons_without_an_uninstall_entry_are_added() {
        let mut r = windows_11();
        r.key(
            r"HKLM\SOFTWARE\Python\PythonCore",
            &[("DisplayName", t("Python Software Foundation"))],
        )
        // installed by the installer of python.org: it has an Uninstall entry
        .key(
            r"HKLM\SOFTWARE\Python\PythonCore\3.12",
            &[
                ("DisplayName", t("Python 3.12 (64-bit)")),
                ("Version", t("3.12.10")),
            ],
        )
        // left over without values
        .key(r"HKLM\SOFTWARE\Python\PythonCore\3.11\InstallPath", &[])
        // by the Python install manager, for one user
        .key(
            &format!(r"{ALICE}\Software\Python\PythonCore\3.14"),
            &[
                ("DisplayName", t("Python 3.14.0")),
                ("Version", t("3.14.0")),
            ],
        )
        .key(
            &format!(r"{ALICE}\Software\Python\PythonCore\3.13-32"),
            &[("Version", t("3.13.9"))],
        )
        // another distribution is not CPython and has its own Uninstall entry
        .key(
            r"HKLM\SOFTWARE\Python\ContinuumAnalytics\Anaconda3",
            &[("Version", t("3.12.4"))],
        );
        let programs = [Item {
            kind: "win",
            id: "{GUID}".into(),
            name: "Python 3.12.10 (64-bit)".into(),
            version: "3.12.10150.0".into(),
            publisher: Some("Python Software Foundation".into()),
            winget_id: None,
            scope: Some("machine"),
        }];
        let mut notes = Vec::new();
        let items = pythons(&r, &programs, &mut notes);
        assert!(notes.is_empty(), "{notes:?}");
        let got: Vec<_> = items
            .iter()
            .map(|i| {
                (
                    i.id.as_str(),
                    i.name.as_str(),
                    i.version.as_str(),
                    i.publisher.as_deref(),
                    i.scope,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (
                    r"Python\PythonCore\3.13-32",
                    "Python 3.13-32",
                    "3.13.9",
                    Some("Python Software Foundation"),
                    Some("user")
                ),
                (
                    r"Python\PythonCore\3.14",
                    "Python 3.14.0",
                    "3.14.0",
                    Some("Python Software Foundation"),
                    Some("user")
                ),
            ]
        );
    }

    #[test]
    fn store_apps_are_written_as_in_the_script() {
        let item = store_item(
            "Microsoft.WindowsTerminal_8wekyb3d8bbwe",
            "Microsoft.WindowsTerminal",
            "1.23.12811.0",
            "CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US",
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(item).unwrap(),
            serde_json::json!({ "type": "appx", "id": "Microsoft.WindowsTerminal_8wekyb3d8bbwe",
                "name": "Microsoft.WindowsTerminal", "version": "1.23.12811.0",
                "publisher": "Microsoft Corporation", "scope": "user" })
        );
        assert_eq!(store_item("", "x", "1", "CN=x"), None);
        assert_eq!(
            common_name(r#"CN="Anthropic, PBC", O="Anthropic, PBC", L=San Francisco, C=US"#),
            "Anthropic, PBC"
        );
        assert_eq!(common_name(r#"CN="A ""B"" C", O=x"#), r#"A "B" C"#);
        assert_eq!(common_name(r#"CN="open"#), "open");
        assert_eq!(
            common_name("CN=24803D75-212C-471A-BC57-9EF86AB91435"),
            "24803D75-212C-471A-BC57-9EF86AB91435"
        );
        assert_eq!(common_name("O=Only"), "O=Only");
        let item = store_item("A_1", "A", "1.0.0.0", "").unwrap();
        assert_eq!(item.publisher, None);
        let item = store_item("A_1", "A\u{7}", "1.0.0.0", &"x".repeat(500)).unwrap();
        assert_eq!(
            (item.name.as_str(), item.publisher.unwrap().len()),
            ("A", 200)
        );
    }

    const BOB: &str = "S-1-5-21-1111-2222-3333-1002";

    #[test]
    fn users_not_signed_in_are_read_from_their_hive_file() {
        let mut r = windows_11();
        r.key(
            &format!(r"{ALICE}\Software\{UNINSTALL}\Zoom"),
            &[
                ("DisplayName", t("Zoom Workplace")),
                ("DisplayVersion", t("6.7.0")),
            ],
        );
        for (sid, dir) in [
            // signed in: read from HKU, the file is not touched
            ("S-1-5-21-1111-2222-3333-1001", r"C:\Users\alice"),
            (BOB, r"%SystemDrive%\Users\bob\"),
            // no file, no access, not a hive
            ("S-1-5-21-1111-2222-3333-1003", r"C:\Users\carol"),
            ("S-1-5-21-1111-2222-3333-1004", r"C:\Users\dave"),
            ("S-1-5-21-1111-2222-3333-1005", r"C:\Users\eve"),
            // not people, or a broken profile kept aside by Windows
            ("S-1-5-18", r"%systemroot%\system32\config\systemprofile"),
            ("S-1-5-21-1111-2222-3333-1006.bak", r"C:\Users\frank"),
        ] {
            r.key(
                &format!(r"{PROFILE_LIST}\{sid}"),
                &[("ProfileImagePath", t(dir))],
            );
        }
        let read = |path: &str| -> std::io::Result<Vec<u8>> {
            use std::io::{Error, ErrorKind};
            match path.rsplit('\\').nth(1).unwrap_or_default() {
                "bob" => {
                    assert!(path.ends_with(r"\Users\bob\NTUSER.DAT"), "{path}");
                    Ok(super::super::hive::build::sample())
                }
                "carol" => Err(Error::from(ErrorKind::NotFound)),
                "dave" => Err(Error::from(ErrorKind::PermissionDenied)),
                "eve" => Ok(b"not a hive".to_vec()),
                _ => panic!("read: {path}"),
            }
        };
        let mut notes = Vec::new();
        let offline = offline_users(&r, &read, &mut notes);
        assert_eq!(offline.keys().collect::<Vec<_>>(), [BOB]);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(
            notes.iter().all(|n| n.contains("a user not signed in")
                && !n.contains("S-1-5")
                && !n.contains("dave")
                && !n.contains("eve")),
            "{notes:?}"
        );
        let registry = WithOffline { live: &r, offline };
        let mut notes = Vec::new();
        let items = programs(&registry, &mut notes);
        assert!(notes.is_empty(), "{notes:?}");
        let got: Vec<_> = items
            .iter()
            .map(|i| (i.id.as_str(), i.name.as_str(), i.version.as_str(), i.scope))
            .collect();
        // the same key name of two users once (the first SID)
        assert_eq!(
            got,
            [
                ("Zoom", "Zoom Workplace", "6.7.0", Some("user")),
                ("Notepad++", "Notepad++ (64-bit x64)", "8.9.9", Some("user")),
            ]
        );
        let pythons = pythons(&registry, &items, &mut notes);
        let got: Vec<_> = pythons
            .iter()
            .map(|i| (i.id.as_str(), i.version.as_str(), i.scope))
            .collect();
        assert_eq!(got, [(r"Python\PythonCore\3.14", "3.14.0", Some("user"))]);
        // the machine is still read from the registry
        assert_eq!(os_info(&registry, "AMD64").unwrap().build, Some(26200));
    }

    #[test]
    fn variables_in_profile_paths_are_expanded() {
        let var = |n: &str| {
            n.eq_ignore_ascii_case("SystemDrive")
                .then(|| "C:".to_owned())
        };
        assert_eq!(expand(r"%SystemDrive%\Users\bob", var), r"C:\Users\bob");
        assert_eq!(expand(r"%systemdrive%\a%Unknown%b", var), r"C:\a%Unknown%b");
        assert_eq!(expand("100% sure", var), "100% sure");
        assert_eq!(expand("%%x", var), "%%x");
        assert_eq!(expand(r"C:\Users\bob", var), r"C:\Users\bob");
    }

    /// The default profile of Windows (copied for every new user, never loaded): a real hive file.
    #[test]
    fn the_hive_of_the_default_profile_is_read() {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let bytes = read_hive_file(&format!(r"{drive}\Users\Default\NTUSER.DAT")).unwrap();
        let hive = Hive::parse(bytes).unwrap();
        let software = hive.subkeys("Software").unwrap().unwrap();
        assert!(software.iter().any(|k| k == "Microsoft"), "{software:?}");
        assert!(
            hive.subkeys(r"Software\Microsoft\Windows\CurrentVersion")
                .unwrap()
                .is_some()
        );
        let desktop = ["WheelScrollLines", "CursorBlinkRate", "MenuShowDelay"]
            .map(|name| hive.value(r"Control Panel\Desktop", name).unwrap());
        assert!(
            desktop
                .iter()
                .any(|v| matches!(v, Some(hive::Data::Text(_)))),
            "{desktop:?}"
        );
        Snapshot::take(&hive, &USER_PLACES).unwrap();
    }

    #[test]
    fn only_plain_files_are_read_as_hives() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_hive_file(dir.path().to_str().unwrap()).is_err());
        let missing = dir.path().join("NTUSER.DAT");
        assert_eq!(
            read_hive_file(missing.to_str().unwrap())
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotFound
        );
        let target = dir.path().join("real.dat");
        std::fs::write(&target, b"regf").unwrap();
        assert_eq!(read_hive_file(target.to_str().unwrap()).unwrap(), b"regf");
        // a link at its place is refused (creating one needs the right to, as in the CI)
        let link = dir.path().join("link.dat");
        if std::os::windows::fs::symlink_file(&target, &link).is_ok() {
            assert_eq!(
                read_hive_file(link.to_str().unwrap())
                    .unwrap_err()
                    .to_string(),
                "not a plain file"
            );
        }
    }

    /// The real inventory of this computer (the developer's or the CI runner).
    #[test]
    fn the_inventory_of_this_machine() {
        let inv = collect().unwrap();
        assert_eq!(inv.os.family, "windows");
        assert!(inv.os.build.unwrap() >= 10240, "{:?}", inv.os);
        assert!(inv.dpkg.is_none());
        assert!(inv.items.iter().any(|i| i.kind == "win"), "no programs");
        let mut ids = BTreeSet::new();
        for item in &inv.items {
            assert!(["win", "appx"].contains(&item.kind), "{item:?}");
            assert!(ids.insert((item.kind, item.id.clone())), "twice: {item:?}");
        }
    }
}

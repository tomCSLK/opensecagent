//! A narrow reader of registry hive files (regf), for the `NTUSER.DAT` of users not signed in (PLAN.md step 3b):
//! keys along a path, their subkey names and their string and number values. The file is read as it is, never
//! loaded into the registry.
//!
//! The file lies in the profile of a user and is read by the service running as SYSTEM, so it is data a user
//! controls: every offset and length is checked, keys are found along a given path only (an index of lists, `ri`, is
//! followed one level), each list is limited to [`MAX_ENTRIES`] and a [`Snapshot`] to a fixed amount of work, so a
//! crafted file cannot loop or make the agent run long. Errors are values, never panics.
//!
//! Not read: the transaction logs (`.LOG1`/`.LOG2`; a hive not written back cleanly is read as it is, perhaps a
//! little behind), big data (`db`, values over 16 KB; not needed for names and versions), classes and security.

use std::collections::{BTreeMap, BTreeSet};

/// Larger files are not read (an `NTUSER.DAT` has a few MB, rarely a hundred).
pub const MAX_FILE: u64 = 256 << 20;
/// At most this many subkeys or values of one key.
const MAX_ENTRIES: usize = 65_536;
/// At most this many keys and values copied into one [`Snapshot`].
const MAX_WORK: usize = 500_000;
/// The base block in front of the hive bins.
const BASE_BLOCK: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HiveError {
    #[error("not a registry hive")]
    NotAHive,
    #[error("the hive is larger than {} MB", MAX_FILE >> 20)]
    TooLarge,
    #[error("the hive is damaged ({0})")]
    Damaged(&'static str),
    #[error("the hive has too many entries")]
    TooMany,
}

/// A value as the inventory needs it: strings (`REG_SZ`, `REG_EXPAND_SZ`) and numbers (`REG_DWORD`, `REG_QWORD`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Data {
    Text(String),
    Number(u64),
}

/// A hive file in memory.
pub struct Hive {
    /// The file from the first hive bin on; cell offsets count from here.
    bins: Vec<u8>,
    root: u32,
}

/// A key node (`nk`): its cell.
#[derive(Clone, Copy)]
struct Key<'a>(&'a [u8]);

impl Hive {
    pub fn parse(mut file: Vec<u8>) -> Result<Self, HiveError> {
        if file.len() as u64 > MAX_FILE {
            return Err(HiveError::TooLarge);
        }
        if file.len() < BASE_BLOCK + 32 || !file.starts_with(b"regf") || u32_at(&file, 0x14)? != 1 {
            return Err(HiveError::NotAHive);
        }
        let root = u32_at(&file, 0x24)?;
        let bins = file.split_off(BASE_BLOCK);
        if !bins.starts_with(b"hbin") {
            return Err(HiveError::Damaged("no hive bin"));
        }
        let hive = Hive { bins, root };
        hive.key(root)?;
        Ok(hive)
    }

    // The inventory reads through a [`Snapshot`]; these two are for the tests.

    /// The names of the subkeys of a key (`path` below the root, separated by `\`, as Windows without regard to
    /// case); `None` if the key does not exist. Damaged subkeys are left out.
    #[cfg(test)]
    pub fn subkeys(&self, path: &str) -> Result<Option<Vec<String>>, HiveError> {
        let Some(key) = self.find(path)? else {
            return Ok(None);
        };
        Ok(Some(
            self.children(key)?
                .into_iter()
                .map(|(name, _, _)| name)
                .collect(),
        ))
    }

    /// A value of a key; `None` if the key or the value does not exist or has a type not read here.
    #[cfg(test)]
    pub fn value(&self, path: &str, name: &str) -> Result<Option<Data>, HiveError> {
        let Some(key) = self.find(path)? else {
            return Ok(None);
        };
        for offset in self.value_offsets(key)? {
            let Ok((value_name, data)) = self.value_at(offset) else {
                continue;
            };
            if same_name(&value_name, name) {
                return Ok(data);
            }
        }
        Ok(None)
    }

    /// The key at `path`, going down from the root one name at a time.
    fn find(&self, path: &str) -> Result<Option<Key<'_>>, HiveError> {
        let mut key = self.key(self.root)?;
        for part in path.split('\\').filter(|p| !p.is_empty()) {
            let mut next = None;
            for offset in self.subkey_offsets(key)? {
                let Ok(child) = self.key(offset) else {
                    continue;
                };
                if child.name().is_ok_and(|n| same_name(&n, part)) {
                    next = Some(child);
                    break;
                }
            }
            match next {
                Some(child) => key = child,
                None => return Ok(None),
            }
        }
        Ok(Some(key))
    }

    /// The subkeys of a key with name, cell offset and node; damaged ones are left out.
    fn children(&self, key: Key<'_>) -> Result<Vec<(String, u32, Key<'_>)>, HiveError> {
        let mut out = Vec::new();
        for offset in self.subkey_offsets(key)? {
            if let Ok(child) = self.key(offset)
                && let Ok(name) = child.name()
            {
                out.push((name, offset, child));
            }
        }
        Ok(out)
    }

    /// The string and number values of a key; damaged ones and other types are left out.
    fn values_of(&self, key: Key<'_>, work: &mut usize) -> Result<Vec<(String, Data)>, HiveError> {
        let mut out = Vec::new();
        for offset in self.value_offsets(key)? {
            spend(work)?;
            if let Ok((name, Some(data))) = self.value_at(offset) {
                out.push((name, data));
            }
        }
        Ok(out)
    }

    /// The data of a cell (without its size field).
    fn cell(&self, offset: u32) -> Result<&[u8], HiveError> {
        let damaged = HiveError::Damaged("a cell outside the file");
        let start = offset as usize;
        let size = i32::from_le_bytes(
            self.bins
                .get(start..start.checked_add(4).ok_or(damaged.clone())?)
                .ok_or(damaged.clone())?
                .try_into()
                .map_err(|_| damaged.clone())?,
        )
        .unsigned_abs() as usize;
        if size < 4 {
            return Err(HiveError::Damaged("a cell without size"));
        }
        let end = start.checked_add(size).ok_or(damaged.clone())?;
        self.bins.get(start + 4..end).ok_or(damaged)
    }

    fn key(&self, offset: u32) -> Result<Key<'_>, HiveError> {
        let cell = self.cell(offset)?;
        if cell.len() < 0x4C || !cell.starts_with(b"nk") {
            return Err(HiveError::Damaged("not a key"));
        }
        Ok(Key(cell))
    }

    /// The cell offsets of the subkeys, from the list of the key (`lf`, `lh`, `li`, or `ri` with lists of those).
    fn subkey_offsets(&self, key: Key<'_>) -> Result<Vec<u32>, HiveError> {
        let mut out = Vec::new();
        if u32_at(key.0, 0x14)? == 0 {
            return Ok(out);
        }
        let list = u32_at(key.0, 0x1C)?;
        let cell = self.cell(list)?;
        if cell.starts_with(b"ri") {
            for at in entries(cell, 4)? {
                self.leaf_list(self.cell(u32_at(cell, at)?)?, &mut out)?;
            }
        } else {
            self.leaf_list(cell, &mut out)?;
        }
        Ok(out)
    }

    /// The offsets of one list of subkeys (not an index).
    fn leaf_list(&self, cell: &[u8], out: &mut Vec<u32>) -> Result<(), HiveError> {
        let step = match cell.get(..2) {
            Some(b"lf" | b"lh") => 8,
            Some(b"li") => 4,
            _ => return Err(HiveError::Damaged("not a list of subkeys")),
        };
        for at in entries(cell, step)? {
            if out.len() >= MAX_ENTRIES {
                return Err(HiveError::TooMany);
            }
            out.push(u32_at(cell, at)?);
        }
        Ok(())
    }

    fn value_offsets(&self, key: Key<'_>) -> Result<Vec<u32>, HiveError> {
        let count = u32_at(key.0, 0x24)? as usize;
        if count == 0 {
            return Ok(Vec::new());
        }
        if count > MAX_ENTRIES {
            return Err(HiveError::TooMany);
        }
        let cell = self.cell(u32_at(key.0, 0x28)?)?;
        (0..count).map(|i| u32_at(cell, i * 4)).collect()
    }

    /// Name and data of a value (`vk`); the data is `None` for types not read here and for big data.
    fn value_at(&self, offset: u32) -> Result<(String, Option<Data>), HiveError> {
        let cell = self.cell(offset)?;
        if cell.len() < 0x14 || !cell.starts_with(b"vk") {
            return Err(HiveError::Damaged("not a value"));
        }
        let name_len = u16_at(cell, 2)? as usize;
        let size = u32_at(cell, 4)?;
        let kind = u32_at(cell, 0x0C)?;
        let compressed = u16_at(cell, 0x10)? & 1 != 0;
        let name = cell
            .get(0x14..0x14 + name_len)
            .ok_or(HiveError::Damaged("a value name"))?;
        let name = decode_name(name, compressed);
        // only the types read here; the others are not even looked up
        if !matches!(kind, 1 | 2 | 4 | 11) {
            return Ok((name, None));
        }
        let data: &[u8] = if size & 0x8000_0000 != 0 {
            // up to 4 bytes in the field of the offset
            let len = (size & 0x7FFF_FFFF) as usize;
            cell.get(8..8 + len)
                .filter(|_| len <= 4)
                .ok_or(HiveError::Damaged("a short value"))?
        } else if size == 0 {
            &[]
        } else {
            let data = self.cell(u32_at(cell, 8)?)?;
            match data.get(..size as usize) {
                Some(data) => data,
                // big data (`db`) is not read
                None if data.starts_with(b"db") => return Ok((name, None)),
                None => return Err(HiveError::Damaged("a value")),
            }
        };
        Ok((name, decode(kind, data)))
    }
}

impl Key<'_> {
    fn name(&self) -> Result<String, HiveError> {
        let len = u16_at(self.0, 0x48)? as usize;
        let compressed = u16_at(self.0, 2)? & 0x20 != 0;
        let bytes = self
            .0
            .get(0x4C..0x4C + len)
            .ok_or(HiveError::Damaged("a key name"))?;
        Ok(decode_name(bytes, compressed))
    }
}

/// The positions of the entries of a list cell: a count at 2, entries of `step` bytes from 4.
fn entries(cell: &[u8], step: usize) -> Result<impl Iterator<Item = usize>, HiveError> {
    let count = u16_at(cell, 2)? as usize;
    if 4 + count * step > cell.len() {
        return Err(HiveError::Damaged("a list longer than its cell"));
    }
    Ok((0..count).map(move |i| 4 + i * step))
}

fn spend(work: &mut usize) -> Result<(), HiveError> {
    *work = work.checked_sub(1).ok_or(HiveError::TooMany)?;
    Ok(())
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, HiveError> {
    bytes
        .get(at..at + 2)
        .and_then(|b| b.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or(HiveError::Damaged("a short record"))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, HiveError> {
    bytes
        .get(at..at + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(HiveError::Damaged("a short record"))
}

/// A name in Latin-1 ("compressed") or UTF-16LE.
fn decode_name(bytes: &[u8], compressed: bool) -> String {
    if compressed {
        bytes.iter().map(|&b| char::from(b)).collect()
    } else {
        utf16(bytes)
    }
}

fn utf16(bytes: &[u8]) -> String {
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .take_while(|&u| u != 0);
    char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

fn decode(kind: u32, data: &[u8]) -> Option<Data> {
    match kind {
        // REG_SZ, REG_EXPAND_SZ (not expanded, as `windows-registry` reads them)
        1 | 2 => Some(Data::Text(utf16(data))),
        // REG_DWORD
        4 => Some(Data::Number(
            u32::from_le_bytes(data.get(..4)?.try_into().ok()?).into(),
        )),
        // REG_QWORD
        11 => Some(Data::Number(u64::from_le_bytes(
            data.get(..8)?.try_into().ok()?,
        ))),
        _ => None,
    }
}

/// Names of keys and values, as Windows compares them: without regard to case.
fn same_name(a: &str, b: &str) -> bool {
    a.chars()
        .flat_map(char::to_uppercase)
        .eq(b.chars().flat_map(char::to_uppercase))
}

fn fold(path: &str) -> String {
    path.split('\\')
        .filter(|p| !p.is_empty())
        .map(str::to_uppercase)
        .collect::<Vec<_>>()
        .join("\\")
}

/// Some keys of a hive copied out, so the file need not stay in memory: each place with its values and the names of
/// its subkeys, and the values of those subkeys (one level), as the inventory reads them.
#[derive(Debug, Default)]
pub struct Snapshot {
    keys: BTreeMap<String, Copied>,
}

#[derive(Debug, Default)]
struct Copied {
    subkeys: Vec<String>,
    values: Vec<(String, Data)>,
}

impl Snapshot {
    /// Copies the places that exist. A damaged subkey is left out, a damaged place is an error.
    pub fn take(hive: &Hive, places: &[&str]) -> Result<Self, HiveError> {
        let mut work = MAX_WORK;
        let mut keys = BTreeMap::new();
        for place in places {
            let Some(key) = hive.find(place)? else {
                continue;
            };
            let children = hive.children(key)?;
            // a list may name the same cell twice; it is copied once
            let mut seen = BTreeSet::new();
            let mut names = Vec::new();
            for (name, offset, child) in children {
                spend(&mut work)?;
                if !seen.insert(offset) {
                    continue;
                }
                match hive.values_of(child, &mut work) {
                    Ok(values) => {
                        let copied = Copied {
                            subkeys: Vec::new(),
                            values,
                        };
                        keys.insert(fold(&format!(r"{place}\{name}")), copied);
                    }
                    Err(HiveError::TooMany) => return Err(HiveError::TooMany),
                    // a damaged entry is left out
                    Err(_) => {}
                }
                names.push(name);
            }
            let values = hive.values_of(key, &mut work)?;
            keys.insert(
                fold(place),
                Copied {
                    subkeys: names,
                    values,
                },
            );
        }
        Ok(Snapshot { keys })
    }

    /// As [`Hive::subkeys`]; the subkeys of a copied subkey are not copied (an empty list).
    pub fn subkeys(&self, path: &str) -> Option<Vec<String>> {
        self.keys.get(&fold(path)).map(|k| k.subkeys.clone())
    }

    pub fn value(&self, path: &str, name: &str) -> Option<Data> {
        self.keys
            .get(&fold(path))?
            .values
            .iter()
            .find(|(n, _)| same_name(n, name))
            .map(|(_, d)| d.clone())
    }
}

/// Hive files for the tests (here and in `windows.rs`).
#[cfg(test)]
pub mod build {
    use super::BASE_BLOCK;

    /// "No cell".
    const NONE: u32 = u32::MAX;

    /// Writes hive files as Windows lays them out (base block, one hive bin, cells of 8-byte steps).
    pub struct Builder {
        bins: Vec<u8>,
    }

    /// A key to write: name, values (name, type, data) and subkeys.
    pub struct K {
        pub name: String,
        pub values: Vec<(String, u32, Vec<u8>)>,
        pub children: Vec<K>,
    }

    pub fn k(name: &str, values: &[(&str, u32, Vec<u8>)], children: Vec<K>) -> K {
        K {
            name: name.into(),
            values: values
                .iter()
                .map(|(n, t, d)| ((*n).into(), *t, d.clone()))
                .collect(),
            children,
        }
    }

    pub fn sz(s: &str) -> Vec<u8> {
        s.encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    pub fn dword(n: u32) -> Vec<u8> {
        n.to_le_bytes().to_vec()
    }

    pub const SZ: u32 = 1;
    pub const EXPAND: u32 = 2;
    pub const DWORD: u32 = 4;
    pub const QWORD: u32 = 11;
    pub const BINARY: u32 = 3;

    impl Builder {
        pub fn new() -> Self {
            let mut bins = vec![0; 32];
            bins[..4].copy_from_slice(b"hbin");
            Builder { bins }
        }

        pub fn cell(&mut self, data: &[u8]) -> u32 {
            let offset = self.bins.len();
            let size = (4 + data.len()).next_multiple_of(8);
            self.bins.extend((-(size as i32)).to_le_bytes());
            self.bins.extend(data);
            self.bins.resize(offset + size, 0);
            offset as u32
        }

        pub fn value(&mut self, name: &str, kind: u32, data: &[u8], ascii: bool) -> u32 {
            let (size, field) = if data.len() <= 4 {
                let mut inline = [0; 4];
                inline[..data.len()].copy_from_slice(data);
                (data.len() as u32 | 0x8000_0000, u32::from_le_bytes(inline))
            } else {
                (data.len() as u32, self.cell(data))
            };
            let name = name_bytes(name, ascii);
            let mut vk = b"vk".to_vec();
            vk.extend((name.len() as u16).to_le_bytes());
            vk.extend(size.to_le_bytes());
            vk.extend(field.to_le_bytes());
            vk.extend(kind.to_le_bytes());
            vk.extend(u16::from(ascii).to_le_bytes());
            vk.extend([0, 0]);
            vk.extend(name);
            self.cell(&vk)
        }

        pub fn key_with(
            &mut self,
            name: &str,
            ascii: bool,
            list: Option<u32>,
            count: u32,
            values: &[u32],
        ) -> u32 {
            let value_list = if values.is_empty() {
                NONE
            } else {
                let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
                self.cell(&bytes)
            };
            let name = name_bytes(name, ascii);
            let mut nk = vec![0; 0x4C];
            nk[..2].copy_from_slice(b"nk");
            nk[2..4].copy_from_slice(&(if ascii { 0x20u16 } else { 0 }).to_le_bytes());
            nk[0x14..0x18].copy_from_slice(&count.to_le_bytes());
            nk[0x1C..0x20].copy_from_slice(&list.unwrap_or(NONE).to_le_bytes());
            nk[0x24..0x28].copy_from_slice(&(values.len() as u32).to_le_bytes());
            nk[0x28..0x2C].copy_from_slice(&value_list.to_le_bytes());
            nk[0x48..0x4A].copy_from_slice(&(name.len() as u16).to_le_bytes());
            nk.extend(name);
            self.cell(&nk)
        }

        /// A list of subkeys of the given kind (`lf`, `lh`, `li`).
        pub fn list(&mut self, sig: &[u8; 2], keys: &[u32]) -> u32 {
            let mut list = sig.to_vec();
            list.extend((keys.len() as u16).to_le_bytes());
            for key in keys {
                list.extend(key.to_le_bytes());
                if sig != b"li" {
                    list.extend([0; 4]);
                }
            }
            self.cell(&list)
        }

        pub fn tree(&mut self, key: &K) -> u32 {
            let children: Vec<u32> = key.children.iter().map(|c| self.tree(c)).collect();
            let values: Vec<u32> = key
                .values
                .iter()
                .map(|(n, t, d)| self.value(n, *t, d, true))
                .collect();
            let list = (!children.is_empty()).then(|| self.list(b"lh", &children));
            self.key_with(&key.name, true, list, children.len() as u32, &values)
        }

        pub fn finish(mut self, root: u32) -> Vec<u8> {
            let len = self.bins.len().next_multiple_of(4096);
            self.bins.resize(len, 0);
            self.bins[8..12].copy_from_slice(&(len as u32).to_le_bytes());
            let mut file = vec![0; BASE_BLOCK];
            file[..4].copy_from_slice(b"regf");
            file[0x14..0x18].copy_from_slice(&1u32.to_le_bytes());
            file[0x18..0x1C].copy_from_slice(&5u32.to_le_bytes());
            file[0x24..0x28].copy_from_slice(&root.to_le_bytes());
            file[0x28..0x2C].copy_from_slice(&(len as u32).to_le_bytes());
            file.extend(self.bins);
            file
        }
    }

    pub fn name_bytes(name: &str, ascii: bool) -> Vec<u8> {
        if ascii {
            name.bytes().collect()
        } else {
            name.encode_utf16().flat_map(u16::to_le_bytes).collect()
        }
    }

    pub const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
    pub const PYTHON: &str = r"Software\Python\PythonCore";

    /// A small `NTUSER.DAT` with two programs and a Python of the Python install manager.
    pub fn sample() -> Vec<u8> {
        let uninstall = k(
            "Uninstall",
            &[],
            vec![
                k(
                    "Zoom",
                    &[
                        ("DisplayName", SZ, sz("Zoom Workplace")),
                        ("DisplayVersion", SZ, sz("6.6.0")),
                        ("Publisher", SZ, sz("Zoom Communications, Inc.")),
                        ("EstimatedSize", DWORD, dword(512_000)),
                        ("InstallLocation", EXPAND, sz(r"%LOCALAPPDATA%\Zoom")),
                        ("Blob", BINARY, vec![1, 2, 3, 4, 5, 6]),
                    ],
                    vec![],
                ),
                k(
                    "Notepad++",
                    &[
                        ("DisplayName", SZ, sz("Notepad++ (64-bit x64)")),
                        // without the terminating NUL
                        (
                            "DisplayVersion",
                            SZ,
                            "8.9.9".encode_utf16().flat_map(u16::to_le_bytes).collect(),
                        ),
                        ("Big", QWORD, 1u64.wrapping_shl(40).to_le_bytes().to_vec()),
                        ("", SZ, sz("default")),
                    ],
                    vec![],
                ),
            ],
        );
        let tree = k(
            "ROOT",
            &[],
            vec![
                k("AppEvents", &[], vec![]),
                k(
                    "Software",
                    &[],
                    vec![
                        k(
                            "Microsoft",
                            &[],
                            vec![k(
                                "Windows",
                                &[],
                                vec![k("CurrentVersion", &[], vec![uninstall])],
                            )],
                        ),
                        k(
                            "Python",
                            &[],
                            vec![k(
                                "PythonCore",
                                &[("DisplayName", SZ, sz("Python Software Foundation"))],
                                vec![k(
                                    "3.14",
                                    &[
                                        ("DisplayName", SZ, sz("Python 3.14.0")),
                                        ("Version", SZ, sz("3.14.0")),
                                    ],
                                    vec![k("InstallPath", &[], vec![])],
                                )],
                            )],
                        ),
                    ],
                ),
            ],
        );
        let mut b = Builder::new();
        let root = b.tree(&tree);
        b.finish(root)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::build::*;
    use super::*;

    fn text(s: &str) -> Option<Data> {
        Some(Data::Text(s.into()))
    }

    #[test]
    fn keys_and_values_are_read_along_a_path() {
        let hive = Hive::parse(sample()).unwrap();
        assert_eq!(
            hive.subkeys(UNINSTALL).unwrap(),
            Some(vec!["Zoom".to_owned(), "Notepad++".to_owned()])
        );
        assert_eq!(
            hive.subkeys("").unwrap(),
            Some(vec!["AppEvents".to_owned(), "Software".to_owned()])
        );
        // names without regard to case, separators as they come
        let zoom = r"\software\MICROSOFT\Windows\CurrentVersion\uninstall\zoom\";
        assert_eq!(
            hive.value(zoom, "displayname").unwrap(),
            text("Zoom Workplace")
        );
        assert_eq!(
            hive.value(zoom, "EstimatedSize").unwrap(),
            Some(Data::Number(512_000))
        );
        // not expanded
        assert_eq!(
            hive.value(zoom, "InstallLocation").unwrap(),
            text(r"%LOCALAPPDATA%\Zoom")
        );
        assert_eq!(hive.value(zoom, "Blob").unwrap(), None);
        assert_eq!(hive.value(zoom, "Missing").unwrap(), None);
        let notepad = format!(r"{UNINSTALL}\Notepad++");
        assert_eq!(
            hive.value(&notepad, "DisplayVersion").unwrap(),
            text("8.9.9")
        );
        assert_eq!(
            hive.value(&notepad, "Big").unwrap(),
            Some(Data::Number(1 << 40))
        );
        assert_eq!(hive.value(&notepad, "").unwrap(), text("default"));
        assert_eq!(hive.subkeys(r"Software\Nothing").unwrap(), None);
        assert_eq!(hive.value(r"Software\Nothing", "x").unwrap(), None);
    }

    #[test]
    fn a_snapshot_keeps_the_places_and_one_level_below() {
        let hive = Hive::parse(sample()).unwrap();
        let snap = Snapshot::take(&hive, &[UNINSTALL, PYTHON, r"Software\Missing"]).unwrap();
        assert_eq!(
            snap.subkeys(&UNINSTALL.to_lowercase()),
            Some(vec!["Zoom".to_owned(), "Notepad++".to_owned()])
        );
        assert_eq!(
            snap.value(&format!(r"{UNINSTALL}\ZOOM"), "DisplayVersion"),
            text("6.6.0")
        );
        assert_eq!(snap.subkeys(PYTHON), Some(vec!["3.14".to_owned()]));
        assert_eq!(
            snap.value(PYTHON, "DisplayName"),
            text("Python Software Foundation")
        );
        assert_eq!(
            snap.value(&format!(r"{PYTHON}\3.14"), "Version"),
            text("3.14.0")
        );
        // two levels below are not copied
        assert_eq!(snap.subkeys(&format!(r"{PYTHON}\3.14")), Some(vec![]));
        assert_eq!(snap.subkeys(&format!(r"{PYTHON}\3.14\InstallPath")), None);
        assert_eq!(snap.subkeys(r"Software\Missing"), None);
        assert_eq!(snap.value(r"Software", "x"), None);
    }

    #[test]
    fn index_lists_short_lists_and_names_in_utf16_are_read() {
        let mut b = Builder::new();
        let name = b.value("DisplayName", SZ, &sz("Ünïcode"), false);
        let a = b.key_with("Äpfel", false, None, 0, &[name]);
        let c = b.key_with("Birnen", true, None, 0, &[]);
        let d = b.key_with("Kirschen", true, None, 0, &[]);
        let first = b.list(b"li", &[a, c]);
        let second = b.list(b"lf", &[d]);
        let mut ri = b"ri".to_vec();
        ri.extend(2u16.to_le_bytes());
        ri.extend(first.to_le_bytes());
        ri.extend(second.to_le_bytes());
        let index = b.cell(&ri);
        let root = b.key_with("ROOT", true, Some(index), 3, &[]);
        let hive = Hive::parse(b.finish(root)).unwrap();
        assert_eq!(
            hive.subkeys("").unwrap(),
            Some(vec![
                "Äpfel".to_owned(),
                "Birnen".to_owned(),
                "Kirschen".to_owned()
            ])
        );
        assert_eq!(hive.value("äPFEL", "displayname").unwrap(), text("Ünïcode"));
    }

    #[test]
    fn an_index_inside_an_index_is_refused() {
        let mut b = Builder::new();
        let leaf = b.key_with("Leaf", true, None, 0, &[]);
        let list = b.list(b"lf", &[leaf]);
        let mut inner = b"ri".to_vec();
        inner.extend(1u16.to_le_bytes());
        inner.extend(list.to_le_bytes());
        let inner = b.cell(&inner);
        let mut outer = b"ri".to_vec();
        outer.extend(1u16.to_le_bytes());
        outer.extend(inner.to_le_bytes());
        let outer = b.cell(&outer);
        let root = b.key_with("ROOT", true, Some(outer), 1, &[]);
        let hive = Hive::parse(b.finish(root)).unwrap();
        assert_eq!(
            hive.subkeys("").unwrap_err(),
            HiveError::Damaged("not a list of subkeys")
        );
    }

    #[test]
    fn other_files_and_broken_hives_are_errors() {
        assert_eq!(Hive::parse(Vec::new()).err(), Some(HiveError::NotAHive));
        assert_eq!(Hive::parse(vec![0; 8192]).err(), Some(HiveError::NotAHive));
        let file = sample();
        // cut in the middle of the base block
        assert_eq!(
            Hive::parse(file[..2048].to_vec()).err(),
            Some(HiveError::NotAHive)
        );
        // another major version
        let mut other = file.clone();
        other[0x14] = 2;
        assert_eq!(Hive::parse(other).err(), Some(HiveError::NotAHive));
        // the root outside the file
        let mut outside = file.clone();
        outside[0x24..0x28].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(Hive::parse(outside), Err(HiveError::Damaged(_))));
        // the root pointing at a value
        let vk = file.windows(2).position(|w| w == b"vk").unwrap() - BASE_BLOCK - 4;
        let mut wrong = file.clone();
        wrong[0x24..0x28].copy_from_slice(&(vk as u32).to_le_bytes());
        assert!(matches!(Hive::parse(wrong), Err(HiveError::Damaged(_))));
        // no hive bin
        let mut no_bin = file;
        no_bin[BASE_BLOCK..BASE_BLOCK + 4].copy_from_slice(b"xxxx");
        assert_eq!(
            Hive::parse(no_bin).err(),
            Some(HiveError::Damaged("no hive bin"))
        );
    }

    #[test]
    fn a_list_naming_the_same_key_many_times_ends_quickly() {
        let mut b = Builder::new();
        let values: Vec<u32> = (0..64)
            .map(|i| b.value(&format!("V{i}"), SZ, &sz("x"), true))
            .collect();
        let child = b.key_with("Same", true, None, 0, &values);
        let many = vec![child; 60_000];
        let list = b.list(b"li", &many);
        let place = b.key_with("Uninstall", true, Some(list), many.len() as u32, &[]);
        let top = b.list(b"lf", &[place]);
        let root = b.key_with("ROOT", true, Some(top), 1, &[]);
        let hive = Hive::parse(b.finish(root)).unwrap();
        let start = Instant::now();
        let snap = Snapshot::take(&hive, &["Uninstall"]).unwrap();
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(snap.subkeys("Uninstall").unwrap().len(), 1);
        assert_eq!(snap.value(r"Uninstall\Same", "V63"), text("x"));
    }

    #[test]
    fn too_many_entries_are_refused() {
        let mut b = Builder::new();
        let child = b.key_with("Same", true, None, 0, &[]);
        let list = b.list(b"li", &vec![child; 40_000]);
        let mut ri = b"ri".to_vec();
        ri.extend(2u16.to_le_bytes());
        ri.extend(list.to_le_bytes());
        ri.extend(list.to_le_bytes());
        let index = b.cell(&ri);
        let root = b.key_with("ROOT", true, Some(index), 80_000, &[]);
        let hive = Hive::parse(b.finish(root)).unwrap();
        assert_eq!(hive.subkeys("").unwrap_err(), HiveError::TooMany);
    }

    // --- fuzz test, as in opensecmon: `FUZZ_ITERATIONS` rounds (a quick run by default), `FUZZ_SEED` the start

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }
    }

    /// Reads everything the inventory would read; the results do not matter, only that it ends without a panic.
    fn read_all(file: Vec<u8>) {
        let Ok(hive) = Hive::parse(file) else {
            return;
        };
        let _ = hive.subkeys("");
        let _ = hive.subkeys(UNINSTALL);
        let _ = hive.value(&format!(r"{UNINSTALL}\Zoom"), "DisplayName");
        let _ = hive.value(&format!(r"{UNINSTALL}\Notepad++"), "Big");
        if let Ok(snap) = Snapshot::take(&hive, &[UNINSTALL, PYTHON, ""]) {
            let _ = snap.subkeys(UNINSTALL);
            let _ = snap.value(&format!(r"{PYTHON}\3.14"), "Version");
        }
    }

    #[test]
    fn fuzz_hive() {
        let rounds = std::env::var("FUZZ_ITERATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3000usize);
        let seed: u64 = std::env::var("FUZZ_SEED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0x5eed_0f5e_c0de);
        let sample = sample();
        let mut rng = Rng(seed | 1);
        for round in 0..rounds {
            let mut file = sample.clone();
            for _ in 0..1 + rng.below(8) {
                // mostly in the cells, sometimes in the base block
                let at = if rng.below(10) == 0 {
                    rng.below(0x30)
                } else {
                    BASE_BLOCK + rng.below(file.len() - BASE_BLOCK)
                };
                match rng.below(5) {
                    0 => file[at] ^= 1 << rng.below(8),
                    1 => file[at] = rng.next() as u8,
                    2 => {
                        // a number: small, near a cell, or large
                        let n = match rng.below(3) {
                            0 => rng.below(16) as u32,
                            1 => rng.below(file.len()) as u32,
                            _ => rng.next() as u32,
                        };
                        let end = (at + 4).min(file.len());
                        file[at..end].copy_from_slice(&n.to_le_bytes()[..end - at]);
                    }
                    3 => file.truncate(at.max(1)),
                    _ => {
                        let len = rng.below(64).min(file.len() - at);
                        let copy = file[at..at + len].to_vec();
                        let to = rng.below(file.len() - len);
                        file[to..to + len].copy_from_slice(&copy);
                    }
                }
                if file.len() <= BASE_BLOCK {
                    break;
                }
            }
            let start = Instant::now();
            let input = file.clone();
            if std::panic::catch_unwind(|| read_all(input)).is_err() {
                panic!("round {round} with FUZZ_SEED={seed} panicked");
            }
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "round {round} with FUZZ_SEED={seed} took {:?}",
                start.elapsed()
            );
        }
    }
}

//! The Flatpak apps of the system and of every user, read from the installation folders.
//!
//! `flatpak list` only sees the installations of the user it runs as, and the agent should not run it as root on
//! folders other users control. So the agent reads the deployed apps itself: `app/<id>/<arch>/<branch>/active`,
//! its `metadata` (proves a deployed app of that ID) and its AppStream file (name and version, the same source
//! `flatpak list` takes them from).
//!
//! Folders of users are their data: every file taken from them must be a regular file owned by that user, so a
//! symlink to a file of root or of another user is skipped, and a FIFO cannot block the agent. Only the ID, the
//! name and the version of an app leave the computer.
//!
//! If the Flatpaks of a user cannot be read (the service lacks `CAP_DAC_READ_SEARCH`, e.g. in a container), the
//! report waits: leaving them out would remove their assets in opensecmon. Homes on a network file system are the
//! exception, as root may never read them there (NFS with `root_squash`): they are named in the journal once and
//! left out.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{InventoryError, Item, field};

const SYSTEM: &str = "/var/lib/flatpak";
const INSTALLATIONS_D: &str = "/etc/flatpak/installations.d";
const PASSWD: &str = "/etc/passwd";
const MOUNTINFO: &str = "/proc/self/mountinfo";
const MOUNTINFO_MAX: u64 = 4 * 1024 * 1024;
const HOMES: &str = "/home";
/// Range of the user IDs of people (as `UID_MIN`/`UID_MAX` in `/etc/login.defs`).
const UID_MIN: u32 = 1000;
const UID_MAX: u32 = 60000;
const HOMES_MAX: usize = 1000;
const APPS_MAX: usize = 2000;
const BRANCHES_MAX: usize = 16;
const METADATA_MAX: u64 = 64 * 1024;
const METAINFO_MAX: u64 = 2 * 1024 * 1024;
/// `O_NONBLOCK` on x86_64 and aarch64 Linux: opening a FIFO returns at once instead of waiting for a writer.
const O_NONBLOCK: i32 = 0o4000;

/// A Flatpak installation: its folder and, for the installation of a user, the user ID its files must belong to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Installation {
    dir: PathBuf,
    user: Option<u32>,
}

/// Homes on a network file system already named in the journal (once per run of the service).
static NAMED: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// The apps of all installations; an error if the Flatpaks of a user on a local file system cannot be read.
pub fn apps() -> Result<Vec<Item>, InventoryError> {
    let list = installations(
        Path::new(SYSTEM),
        Path::new(INSTALLATIONS_D),
        Path::new(PASSWD),
        Path::new(HOMES),
    );
    let mountinfo = read_file(Path::new(MOUNTINFO), None, MOUNTINFO_MAX).unwrap_or_default();
    let (items, skipped) = apps_of(&list, &mountinfo)
        .map_err(|e| InventoryError::Read("the Flatpaks of the users", e))?;
    let mut named = NAMED.lock().unwrap_or_else(|e| e.into_inner());
    for (dir, why) in skipped {
        if named.insert(dir.clone()) {
            println!(
                "the Flatpaks in {} are left out: {why} (a home on a network file system, which root may not read)",
                dir.display()
            );
        }
    }
    Ok(items)
}

/// User installations left out, with the reason.
type Skipped = Vec<(PathBuf, String)>;

/// The apps of the installations, and the user installations left out because they are on a network file system
/// and cannot be read.
fn apps_of(list: &[Installation], mountinfo: &str) -> Result<(Vec<Item>, Skipped), String> {
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    for inst in list {
        if inst.user.is_some() {
            match has_apps(&inst.dir) {
                Ok(false) => continue,
                Ok(true) => {}
                Err(e) if mount_type(mountinfo, &inst.dir).is_some_and(|t| is_network_fs(&t)) => {
                    skipped.push((inst.dir.clone(), e.to_string()));
                    continue;
                }
                Err(e) => {
                    return Err(format!(
                        "{}: {e}; leaving them out would remove them from opensecmon (run it as root; the \
                         service needs the capability CAP_DAC_READ_SEARCH, in a container --cap-add DAC_READ_SEARCH)",
                        inst.dir.display()
                    ));
                }
            }
        }
        items.extend(apps_in(inst));
    }
    Ok((items, skipped))
}

/// Whether an installation of a user has an `app` folder the agent can list; `Ok(false)` if there is none.
fn has_apps(dir: &Path) -> io::Result<bool> {
    let absent = |e: &io::Error| {
        matches!(
            e.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
        )
    };
    match fs::symlink_metadata(dir) {
        Err(e) if absent(&e) => return Ok(false),
        Err(e) => return Err(e),
        Ok(_) => {}
    }
    match fs::read_dir(dir.join("app")) {
        Ok(_) => Ok(true),
        Err(e) if absent(&e) => Ok(false),
        Err(e) => Err(e),
    }
}

/// The file system type of the mount `path` is on, from `/proc/self/mountinfo` (the longest mount point that
/// contains it).
fn mount_type(mountinfo: &str, path: &Path) -> Option<String> {
    mountinfo
        .lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let point = unescape_mount(left.split(' ').nth(4)?);
            let fstype = right.split(' ').next()?;
            path.starts_with(&point)
                .then(|| (point.components().count(), fstype.to_owned()))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, fstype)| fstype)
}

/// Mount points in `mountinfo` write space, tab, newline and backslash as octal escapes (`\040`).
fn unescape_mount(s: &str) -> PathBuf {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && let Some(code) = s
                .get(i + 1..i + 4)
                .and_then(|o| u8::from_str_radix(o, 8).ok())
        {
            out.push(code);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    PathBuf::from(String::from_utf8_lossy(&out).into_owned())
}

fn is_network_fs(fstype: &str) -> bool {
    let base = fstype.strip_prefix("fuse.").unwrap_or(fstype);
    [
        "nfs",
        "nfs4",
        "cifs",
        "smb3",
        "smbfs",
        "ceph",
        "glusterfs",
        "afs",
        "lustre",
        "gpfs",
        "beegfs",
        "9p",
        "sshfs",
        "davfs",
        "ocfs2",
        "gfs2",
    ]
    .contains(&base)
}

/// The system installation, further system installations (`installations.d`) and the one of every user.
fn installations(
    system: &Path,
    installations_d: &Path,
    passwd: &Path,
    homes: &Path,
) -> Vec<Installation> {
    let mut list = vec![Installation {
        dir: system.to_owned(),
        user: None,
    }];
    list.extend(
        extra_installations(installations_d)
            .into_iter()
            .map(|dir| Installation { dir, user: None }),
    );
    list.extend(
        user_homes(passwd, homes)
            .into_iter()
            .map(|(home, uid)| Installation {
                dir: home.join(".local/share/flatpak"),
                user: Some(uid),
            }),
    );
    let mut seen = std::collections::BTreeSet::new();
    list.retain(|i| seen.insert(i.dir.clone()));
    list
}

/// The `Path=` of the `[Installation "…"]` sections in `/etc/flatpak/installations.d/*.conf`.
fn extra_installations(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "conf"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for file in files.iter().take(100) {
        let Some(text) = read_file(file, None, METADATA_MAX) else {
            continue;
        };
        let mut in_installation = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                in_installation = line.starts_with("[Installation \"");
            } else if in_installation
                && let Some(path) = line.strip_prefix("Path=")
                && path.starts_with('/')
            {
                out.push(PathBuf::from(path));
            }
        }
    }
    out
}

/// Homes of people: from `/etc/passwd`, and the folders in `/home` (users from a directory service are not in
/// `passwd`), with the user ID their files belong to.
fn user_homes(passwd: &Path, homes: &Path) -> Vec<(PathBuf, u32)> {
    let mut found: BTreeMap<PathBuf, u32> = BTreeMap::new();
    if let Some(text) = read_file(passwd, None, 4 * 1024 * 1024) {
        for line in text.lines() {
            let fields: Vec<&str> = line.split(':').collect();
            let [_, _, uid, _, _, home, ..] = fields.as_slice() else {
                continue;
            };
            if let Ok(uid) = uid.parse::<u32>()
                && (UID_MIN..UID_MAX).contains(&uid)
                && home.starts_with('/')
                && *home != "/"
            {
                found.entry(PathBuf::from(home)).or_insert(uid);
            }
        }
    }
    if let Ok(entries) = fs::read_dir(homes) {
        for entry in entries.flatten().take(HOMES_MAX) {
            if let Ok(md) = entry.path().symlink_metadata()
                && md.is_dir()
                && (UID_MIN..UID_MAX).contains(&md.uid())
            {
                found.entry(entry.path()).or_insert(md.uid());
            }
        }
    }
    found.into_iter().take(HOMES_MAX).collect()
}

/// The deployed apps of an installation.
fn apps_in(inst: &Installation) -> Vec<Item> {
    let mut out = Vec::new();
    for id in names(&inst.dir.join("app"), APPS_MAX) {
        if !is_flatpak_id(&id) {
            continue;
        }
        let app = inst.dir.join("app").join(&id);
        for arch in names(&app, BRANCHES_MAX) {
            if arch == "current" {
                continue;
            }
            for branch in names(&app.join(&arch), BRANCHES_MAX) {
                let active = app.join(&arch).join(&branch).join("active");
                if let Some(item) = deployed(&active, &id, inst.user) {
                    out.push(item);
                }
            }
        }
    }
    out
}

/// The app deployed in `active`, if its `metadata` names it.
fn deployed(active: &Path, id: &str, user: Option<u32>) -> Option<Item> {
    let metadata = read_file(&active.join("metadata"), user, METADATA_MAX)?;
    let mut lines = metadata.lines().map(str::trim);
    if !lines.any(|l| l == "[Application]") || !lines.any(|l| l == format!("name={id}")) {
        return None;
    }
    let share = active.join("files/share");
    let (name, version) = [
        share.join(format!("metainfo/{id}.metainfo.xml")),
        share.join(format!("metainfo/{id}.appdata.xml")),
        share.join(format!("appdata/{id}.appdata.xml")),
    ]
    .iter()
    .find_map(|p| read_file(p, user, METAINFO_MAX))
    .map(|xml| metainfo(&xml))
    .unwrap_or_default();
    Some(Item {
        kind: "flatpak",
        id: id.to_owned(),
        name: name.unwrap_or_else(|| id.to_owned()),
        version: version.unwrap_or_default(),
        publisher: None,
        winget_id: None,
        scope: Some(if user.is_some() { "user" } else { "machine" }),
    })
}

/// The names in a folder that are folders themselves or links (`active` is one), sorted; empty if unreadable.
fn names(dir: &Path, max: usize) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .take(max)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    out.sort();
    out
}

/// A regular file of at most `max` bytes, owned by `user` if given; anything else is `None`.
fn read_file(path: &Path, user: Option<u32>, max: u64) -> Option<String> {
    let file: File = OpenOptions::new()
        .read(true)
        .custom_flags(O_NONBLOCK)
        .open(path)
        .ok()?;
    let md = file.metadata().ok()?;
    if !md.is_file() || md.len() > max || user.is_some_and(|uid| md.uid() != uid) {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(max).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// A Flatpak application ID (reverse DNS: `org.mozilla.firefox`).
fn is_flatpak_id(s: &str) -> bool {
    s.len() <= 255
        && s.contains('.')
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Name and version from an AppStream file: the `<name>` of the component without `xml:lang`, and the version of
/// the newest `<release>` (by `timestamp` or `date`, otherwise the first, as AppStream lists them newest first).
fn metainfo(xml: &str) -> (Option<String>, Option<String>) {
    let mut stack: Vec<&str> = Vec::new();
    let mut name: Option<String> = None;
    // (date or timestamp as a sortable key, version), first one wins on equal keys.
    let mut newest: Option<(String, String)> = None;
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt..];
        let skip = [
            ("<!--", "-->"),
            ("<![CDATA[", "]]>"),
            ("<?", "?>"),
            ("<!", ">"),
        ]
        .iter()
        .find(|(open, _)| rest.starts_with(open));
        if let Some((_, close)) = skip {
            match rest.find(close) {
                Some(end) => {
                    rest = &rest[end + close.len()..];
                    continue;
                }
                None => break,
            }
        }
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[1..end];
        rest = &rest[end + 1..];
        if let Some(closing) = tag.strip_prefix('/') {
            if let Some(pos) = stack.iter().rposition(|s| *s == closing.trim()) {
                stack.truncate(pos);
            }
            continue;
        }
        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let (tag_name, attrs) = tag
            .split_once(|c: char| c.is_ascii_whitespace())
            .unwrap_or((tag, ""));
        let parent = stack.last().copied();
        if tag_name == "name"
            && parent == Some("component")
            && name.is_none()
            && !self_closing
            && attr(attrs, "xml:lang").is_none()
        {
            let text = &rest[..rest.find('<').unwrap_or(rest.len())];
            name = Some(field(&unescape(text), 200)).filter(|n| !n.is_empty());
        }
        if tag_name == "release"
            && parent == Some("releases")
            && let Some(version) = attr(attrs, "version")
        {
            let version = field(&unescape(version), 100);
            let key = attr(attrs, "timestamp")
                .and_then(|t| t.parse::<u64>().ok())
                .map(|t| format!("t{t:020}"))
                .or_else(|| attr(attrs, "date").map(|d| format!("d{}", field(d, 40))))
                .unwrap_or_default();
            let newer = newest.as_ref().is_none_or(|(k, _)| key > *k);
            if !version.is_empty() && newer {
                newest = Some((key, version));
            }
        }
        if !self_closing {
            stack.push(tag_name);
            if stack.len() > 64 {
                break;
            }
        }
    }
    (name, newest.map(|(_, v)| v))
}

/// The value of an attribute in the attribute part of a tag (`a="1" b='2'`).
fn attr<'a>(attrs: &'a str, key: &str) -> Option<&'a str> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start();
        let eq = rest.find('=')?;
        let k = rest[..eq].trim();
        rest = rest[eq + 1..].trim_start();
        let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let end = rest[1..].find(quote)? + 1;
        let value = &rest[1..end];
        rest = &rest[end + 1..];
        if k == key {
            return Some(value);
        }
    }
}

/// XML entities in text and attribute values.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e => e
                .strip_prefix("#x")
                .map(|h| u32::from_str_radix(h, 16))
                .or_else(|| e.strip_prefix('#').map(str::parse::<u32>))
                .and_then(Result::ok)
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAWING: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- Copyright -->
<component type="desktop-application">
  <id>com.github.maoschanz.drawing</id>
  <name>Drawing</name>
  <name xml:lang="tr">Çizim</name>
  <developer id="io.github.maoschanz"><name>Romain F. T.</name></developer>
  <description><p>Edit &lt;screenshots&gt; &amp; memes</p></description>
  <releases>
    <release version="1.0.2" date="2023-01-03"><description><p>Fixes</p></description></release>
    <release version="1.0.1" date="2022-12-24"/>
  </releases>
</component>"#;

    #[test]
    fn name_and_version_come_from_the_appstream_file() {
        assert_eq!(
            metainfo(DRAWING),
            (Some("Drawing".into()), Some("1.0.2".into()))
        );
        // The developer's name comes first here; the component's own name counts.
        let xml = r#"<component><developer_name>ACME</developer_name><developer><name>ACME Inc.</name></developer>
            <name xml:lang="de">Werkzeug</name><name>Tool &amp; More</name>
            <releases><release version="2.0" timestamp="1700000000"/><release version="2.1" timestamp="1710000000"/>
            <release version="1.0"/></releases></component>"#;
        assert_eq!(
            metainfo(xml),
            (Some("Tool & More".into()), Some("2.1".into()))
        );
        // Releases in an older order by date, CDATA and comments in between.
        let xml = "<component><![CDATA[<name>no</name>]]><!-- <name>no</name> --><name>App</name><releases>\
                   <release date='2020-01-01' version='1'/><release date='2024-05-01' version='3'/></releases></component>";
        assert_eq!(metainfo(xml), (Some("App".into()), Some("3".into())));
        assert_eq!(metainfo(""), (None, None));
        assert_eq!(metainfo("<component><name>"), (None, None));
        assert_eq!(metainfo("not xml at all < > &"), (None, None));
        // A version with control characters is cleaned, an overlong one cut.
        let xml = format!(
            "<component><releases><release version=\"{}\x07\"/></releases></component>",
            "9".repeat(300)
        );
        assert_eq!(metainfo(&xml).1.unwrap().len(), 100);
    }

    #[test]
    fn entities_and_attributes_are_read() {
        assert_eq!(
            unescape("a &amp; b &lt;c&gt; &#65;&#x42; &bogus; &"),
            "a & b <c> AB &bogus; &"
        );
        assert_eq!(attr(r#"a="1" b='two' c = "3""#, "b"), Some("two"));
        assert_eq!(attr(r#"a="1" c = "3""#, "c"), Some("3"));
        assert_eq!(attr(r#"a="1"#, "a"), None);
        assert_eq!(attr("", "a"), None);
    }

    /// Writes a deployed app like flatpak does.
    fn deploy(root: &Path, id: &str, branch: &str, metainfo_xml: Option<&str>) -> PathBuf {
        let commit = root.join(format!("app/{id}/x86_64/{branch}/0123abcd"));
        fs::create_dir_all(commit.join("files/share/metainfo")).unwrap();
        fs::write(
            commit.join("metadata"),
            format!("[Application]\nname={id}\nruntime=org.gnome.Platform/x86_64/50\n"),
        )
        .unwrap();
        if let Some(xml) = metainfo_xml {
            fs::write(
                commit.join(format!("files/share/metainfo/{id}.metainfo.xml")),
                xml,
            )
            .unwrap();
        }
        std::os::unix::fs::symlink(
            "0123abcd",
            root.join(format!("app/{id}/x86_64/{branch}/active")),
        )
        .unwrap();
        let _ = std::os::unix::fs::symlink("x86_64/stable", root.join(format!("app/{id}/current")));
        commit
    }

    fn my_uid(dir: &Path) -> u32 {
        dir.metadata().unwrap().uid()
    }

    #[test]
    fn deployed_apps_of_an_installation() {
        let dir = tempfile::tempdir().unwrap();
        deploy(
            dir.path(),
            "com.github.maoschanz.drawing",
            "stable",
            Some(DRAWING),
        );
        deploy(dir.path(), "org.example.NoInfo", "stable", None);
        deploy(dir.path(), "org.example.NoInfo", "beta", None);
        // Not an app: wrong ID in the metadata, an invalid ID, a stray file.
        let c = deploy(dir.path(), "org.example.Other", "stable", None);
        fs::write(c.join("metadata"), "[Runtime]\nname=org.example.Other\n").unwrap();
        fs::create_dir_all(dir.path().join("app/not-an-id")).unwrap();
        fs::write(dir.path().join("app/org.example.file"), "").unwrap();

        let system = Installation {
            dir: dir.path().to_owned(),
            user: None,
        };
        let items = apps_in(&system);
        let ids: Vec<(&str, &str, &str)> = items
            .iter()
            .map(|i| (i.id.as_str(), i.name.as_str(), i.version.as_str()))
            .collect();
        assert_eq!(
            ids,
            [
                ("com.github.maoschanz.drawing", "Drawing", "1.0.2"),
                ("org.example.NoInfo", "org.example.NoInfo", ""),
                ("org.example.NoInfo", "org.example.NoInfo", ""),
            ]
        );
        assert!(items.iter().all(|i| i.scope == Some("machine")));
        assert_eq!(
            super::super::sorted(items).len(),
            2,
            "the same app twice counts once"
        );

        let user = Installation {
            dir: dir.path().to_owned(),
            user: Some(my_uid(dir.path())),
        };
        assert!(apps_in(&user).iter().all(|i| i.scope == Some("user")));
        assert_eq!(apps_in(&user).len(), 3);
        // Files of another user are not taken from a user's installation.
        let other = Installation {
            dir: dir.path().to_owned(),
            user: Some(my_uid(dir.path()) + 1),
        };
        assert!(apps_in(&other).is_empty());
        assert!(
            apps_in(&Installation {
                dir: dir.path().join("missing"),
                user: None
            })
            .is_empty()
        );
    }

    #[test]
    fn links_to_files_of_others_and_fifos_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let uid = my_uid(dir.path());
        let c = deploy(dir.path(), "org.example.App", "stable", None);
        let user = Installation {
            dir: dir.path().to_owned(),
            user: Some(uid),
        };
        // The AppStream file points to a file of root: the app stays, the file is not read.
        let target = c.join("files/share/metainfo/org.example.App.metainfo.xml");
        std::os::unix::fs::symlink("/etc/passwd", &target).unwrap();
        let items = apps_in(&user);
        assert_eq!(
            (items[0].name.as_str(), items[0].version.as_str()),
            ("org.example.App", "")
        );
        // The metadata itself points to a file of root: no app at all.
        fs::remove_file(c.join("metadata")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", c.join("metadata")).unwrap();
        assert!(apps_in(&user).is_empty());
        // A FIFO in place of the metadata does not block.
        fs::remove_file(c.join("metadata")).unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(c.join("metadata"))
            .status()
            .unwrap();
        assert!(made.success());
        assert!(apps_in(&user).is_empty());
    }

    #[test]
    fn installations_of_the_system_and_the_users() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let uid = my_uid(root);
        fs::create_dir_all(root.join("installations.d")).unwrap();
        fs::write(
            root.join("installations.d/extra.conf"),
            "[Installation \"extra\"]\nPath=/opt/flatpak\nDisplayName=Extra\n\n[Other]\nPath=/nope\n",
        )
        .unwrap();
        fs::write(
            root.join("installations.d/ignored.txt"),
            "[Installation \"x\"]\nPath=/x\n",
        )
        .unwrap();
        fs::write(
            root.join("passwd"),
            "root:x:0:0:root:/root:/bin/bash\nanna:x:1001:1001::/srv/anna:/bin/bash\n\
                 nobody:x:65534:65534::/nonexistent:/usr/sbin/nologin\nbad line\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("home/ben")).unwrap();
        let list = installations(
            Path::new("/var/lib/flatpak"),
            &root.join("installations.d"),
            &root.join("passwd"),
            &root.join("home"),
        );
        let mut expected = vec![
            Installation {
                dir: "/var/lib/flatpak".into(),
                user: None,
            },
            Installation {
                dir: "/opt/flatpak".into(),
                user: None,
            },
            Installation {
                dir: "/srv/anna/.local/share/flatpak".into(),
                user: Some(1001),
            },
        ];
        // The test folder belongs to the user running the tests; in /home it counts only in the range of people.
        if (UID_MIN..UID_MAX).contains(&uid) {
            expected.push(Installation {
                dir: root.join("home/ben/.local/share/flatpak"),
                user: Some(uid),
            });
            expected[2..].sort_by(|a, b| a.dir.cmp(&b.dir));
        }
        assert_eq!(list, expected);
    }

    #[test]
    fn the_mount_of_a_home_is_found() {
        let info = "22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw\n\
                    40 22 0:50 / /home rw shared:20 - autofs systemd-1 rw\n\
                    41 40 0:51 /export/home /home/anna rw - nfs4 server:/export/home rw\n\
                    42 22 0:52 / /mnt/my\\040disk rw - cifs //nas/share rw\n\
                    bad line\n";
        assert_eq!(
            mount_type(info, Path::new("/home/anna/.local/share/flatpak")).as_deref(),
            Some("nfs4")
        );
        assert_eq!(
            mount_type(info, Path::new("/home/annabel/x")).as_deref(),
            Some("autofs")
        );
        assert_eq!(
            mount_type(info, Path::new("/var/lib/flatpak")).as_deref(),
            Some("ext4")
        );
        assert_eq!(
            mount_type(info, Path::new("/mnt/my disk/x")).as_deref(),
            Some("cifs")
        );
        assert_eq!(mount_type("", Path::new("/home")), None);
        assert!(is_network_fs("nfs4") && is_network_fs("fuse.sshfs") && is_network_fs("cifs"));
        assert!(
            !is_network_fs("ext4") && !is_network_fs("autofs") && !is_network_fs("fuse.gvfsd-fuse")
        );
    }

    #[test]
    fn unreadable_flatpaks_of_a_user_hold_the_report_back() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home/anna");
        let inst = home.join(".local/share/flatpak");
        deploy(&inst, "org.example.Tool", "stable", None);
        let uid = my_uid(dir.path());
        let list = [
            Installation {
                dir: dir.path().join("home/ben/.local/share/flatpak"),
                user: Some(uid),
            },
            Installation {
                dir: inst.clone(),
                user: Some(uid),
            },
        ];
        let (items, skipped) = apps_of(&list, "").unwrap();
        assert_eq!(items.len(), 1);
        assert!(skipped.is_empty());

        fs::set_permissions(&home, fs::Permissions::from_mode(0o000)).unwrap();
        let unreadable = fs::read_dir(&inst).is_err();
        if unreadable {
            // On a local file system the report waits.
            let e = apps_of(&list, "").unwrap_err();
            assert!(e.contains("CAP_DAC_READ_SEARCH"), "{e}");
            // On a network file system the home is left out and named.
            let info = format!(
                "41 1 0:51 / {} rw - nfs4 server:/home rw\n",
                dir.path().join("home").display()
            );
            let (items, skipped) = apps_of(&list, &info).unwrap();
            assert!(items.is_empty());
            assert_eq!(skipped.len(), 1);
            assert_eq!(skipped[0].0, inst);
        }
        fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).unwrap();
        // Run as root (which reads anything) the check above cannot happen.
        assert!(
            unreadable || std::env::var("USER").as_deref() == Ok("root") || my_uid(dir.path()) == 0
        );
    }

    /// On the developer's machine: the same apps as `flatpak list` shows for the system installation.
    #[test]
    fn the_same_as_flatpak_list() {
        let Ok(Some(out)) = super::super::linux::run(
            "flatpak",
            &[
                "list",
                "--app",
                "--system",
                "--columns=application,name,version",
            ],
            1024 * 1024,
        ) else {
            return;
        };
        let mine = apps_in(&Installation {
            dir: SYSTEM.into(),
            user: None,
        });
        for line in out.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let [id, name, version] = f.as_slice() else {
                continue;
            };
            assert!(
                mine.iter()
                    .any(|i| i.id == *id && i.name == *name && i.version == *version),
                "{line:?} not in {mine:?}"
            );
        }
    }
}

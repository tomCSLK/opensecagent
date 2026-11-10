//! What the computer has installed, in the form of the JSON report of the agent API.
//!
//! On Debian, Ubuntu and systems based on them: `/etc/os-release`, the running kernel, a pending reboot, Ubuntu
//! Pro, the package list of dpkg, the snaps and the Flatpak apps of the system and of every user. On Windows: the
//! version of Windows, the installed programs and the store apps (`windows.rs`).

use serde::Serialize;

#[cfg(target_os = "linux")]
mod flatpak;
#[cfg(any(windows, test))]
mod hive;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod snap;
#[cfg(windows)]
mod windows;

/// The system, as `os` of the report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Os {
    /// `linux` or `windows`.
    pub family: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_release: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reboot_required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ubuntu_pro: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ubr: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installation_type: Option<String>,
    /// Machine architecture as the system names it (`x86_64`, `aarch64`, `x64`, `arm64`).
    pub arch: String,
}

/// Software outside dpkg (snaps, Flatpaks, Windows programs, store apps), as `items` of the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub winget_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<&'static str>,
}

/// Everything a report carries about the computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    pub hostname: String,
    pub os: Os,
    /// Linux with dpkg: the package lines as `dpkg-query` writes them.
    pub dpkg: Option<String>,
    pub items: Vec<Item>,
}

#[derive(Debug, thiserror::Error)]
pub enum InventoryError {
    #[error("this system is not supported yet: {0}")]
    #[cfg_attr(
        any(target_os = "linux", windows),
        allow(dead_code, reason = "other systems")
    )]
    Unsupported(&'static str),
    #[error("{0} could not be read: {1}")]
    Read(&'static str, String),
}

/// Reads the inventory of this computer.
pub fn collect() -> Result<Inventory, InventoryError> {
    #[cfg(target_os = "linux")]
    return linux::collect();
    #[cfg(windows)]
    return windows::collect();
    #[cfg(not(any(target_os = "linux", windows)))]
    return Err(InventoryError::Unsupported(
        "only Linux and Windows are supported",
    ));
}

/// The items in a fixed order without duplicates, so the same software always gives the same report (and hash).
pub fn sorted(mut items: Vec<Item>) -> Vec<Item> {
    items.sort_by(|a, b| {
        (a.kind, &a.id, &a.version, &a.name, a.scope)
            .cmp(&(b.kind, &b.id, &b.version, &b.name, b.scope))
    });
    items.dedup();
    items
}

/// Text from the system for a field of the report: no control characters, trimmed, at most `max` characters.
pub fn field(text: &str, max: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(max)
        .collect()
}

/// The host name for opensecmon: letters, digits, `.` and `-`, at most 253 characters (as the scripts send it).
pub fn clean_hostname(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        .take(253)
        .collect()
}

/// The host name of this computer (empty if unknown; opensecmon then shows only the name the user gave).
pub fn hostname() -> String {
    #[cfg(target_os = "linux")]
    let raw = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    #[cfg(not(target_os = "linux"))]
    let raw = std::env::var("COMPUTERNAME").unwrap_or_default();
    clean_hostname(raw.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_names_are_cleaned() {
        assert_eq!(clean_hostname("laptop"), "laptop");
        assert_eq!(clean_hostname("PC-WOHNZIMMER"), "PC-WOHNZIMMER");
        assert_eq!(clean_hostname("a b\"c\n$(x)"), "abcx");
        assert_eq!(clean_hostname(&"a".repeat(300)).len(), 253);
    }

    #[test]
    fn the_os_is_written_as_the_api_describes_it() {
        let os = Os {
            family: "linux",
            os_release: Some("ID=debian\n".into()),
            kernel: Some("6.12.48+deb13-amd64".into()),
            reboot_required: Some(false),
            ubuntu_pro: Some(false),
            arch: "x86_64".into(),
            ..Os::default()
        };
        assert_eq!(
            serde_json::to_value(os).unwrap(),
            serde_json::json!({ "family": "linux", "os_release": "ID=debian\n",
                "kernel": "6.12.48+deb13-amd64", "reboot_required": false, "ubuntu_pro": false,
                "arch": "x86_64" })
        );
        let item = Item {
            kind: "flatpak",
            id: "org.mozilla.firefox".into(),
            name: "Firefox".into(),
            version: "144.0.2".into(),
            publisher: None,
            winget_id: None,
            scope: Some("user"),
        };
        assert_eq!(
            serde_json::to_value(item).unwrap(),
            serde_json::json!({ "type": "flatpak", "id": "org.mozilla.firefox", "name": "Firefox",
                "version": "144.0.2", "scope": "user" })
        );
    }
}

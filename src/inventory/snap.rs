//! The installed snaps, from `snap list` as the script reads them.

use super::linux::run;
use super::{InventoryError, Item, field};

/// `snap list` of a desktop has a few dozen lines.
const OUTPUT_MAX: u64 = 1024 * 1024;

/// The snaps; none if snapd is not installed. If `snap` is there but fails, the report waits: an empty list would
/// remove the snaps from opensecmon until the next report.
pub fn snaps() -> Result<Vec<Item>, InventoryError> {
    match run(
        "snap",
        &["list", "--unicode=never", "--color=never"],
        OUTPUT_MAX,
    ) {
        Ok(Some(out)) => Ok(parse(&out)),
        Ok(None) => Ok(Vec::new()),
        Err(e) => Err(InventoryError::Read("the snaps (snap list)", e)),
    }
}

/// The lines of `snap list` after the header: name, version, revision, tracking, publisher (a trailing `*` or
/// `**` marks a verified one, `-` none), notes.
fn parse(output: &str) -> Vec<Item> {
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [name, version, _rev, _tracking, publisher, ..] = fields.as_slice() else {
                return None;
            };
            if !is_snap_name(name) {
                return None;
            }
            let publisher = match publisher.trim_end_matches('*') {
                "-" | "" => None,
                p => Some(field(p, 200)),
            };
            Some(Item {
                kind: "snap",
                id: (*name).to_owned(),
                name: (*name).to_owned(),
                version: field(version, 100),
                publisher,
                winget_id: None,
                scope: None,
            })
        })
        .collect()
}

/// A snap name as the store allows it (lower case letters, digits, hyphens).
fn is_snap_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_of_snap_list_become_items() {
        let out = "Name      Version          Rev    Tracking         Publisher   Notes\n\
                   core22    20250923         2139   latest/stable    canonical** base\n\
                   firefox   144.0.2-1        7177   latest/stable/…  mozilla**   -\n\
                   mytool    1.0              x1     -                -           -\n\
                   Bad_Name  1.0              1      latest/stable    someone     -\n\
                   short line\n";
        let items = parse(out);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].id, "core22");
        assert_eq!(items[1].version, "144.0.2-1");
        assert_eq!(items[1].publisher.as_deref(), Some("mozilla"));
        assert_eq!(items[2].publisher, None);
        assert!(items.iter().all(|i| i.kind == "snap" && i.scope.is_none()));
        assert!(parse("").is_empty());
    }
}

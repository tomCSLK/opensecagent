//! Embeds the icon and the version information into opensecagent.exe on Windows. Nothing to do on Linux.

fn main() {
    #[cfg(windows)]
    windows_resources();
}

#[cfg(windows)]
fn windows_resources() {
    const ICON_PATH: &str = "packaging/windows/opensecagent.ico";
    println!("cargo:rerun-if-changed={ICON_PATH}");
    // only when building for Windows, not when checking for another target on Windows
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon(ICON_PATH)
        .set("ProductName", "opensecagent")
        .set("FileDescription", "opensecagent")
        .set("CompanyName", "Thomas Cieslik")
        .set(
            "LegalCopyright",
            "Copyright (C) 2026 Thomas Cieslik, AGPL-3.0-or-later",
        )
        .set("OriginalFilename", "opensecagent.exe")
        .set("InternalName", "opensecagent");
    if let Err(err) = res.compile() {
        panic!("embedding the Windows resources failed: {err}");
    }
}

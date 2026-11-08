//! Embeds the icon, the version information and the manifest into the programs on Windows. Nothing to do on Linux.

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

    // The manifest (common controls 6 for the task dialogs, scaling) for every program and test; the start-menu
    // program opensecagent-enroll.exe also asks for administrator rights, opensecagent.exe runs with the rights
    // of whoever starts it.
    const MANIFEST: &str = "packaging/windows/opensecagent.manifest";
    println!("cargo:rerun-if-changed={MANIFEST}");
    // options of the Microsoft linker
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let manifest =
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"))
            .join(MANIFEST);
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    println!(
        "cargo:rustc-link-arg-bin=opensecagent-enroll=/MANIFESTUAC:level='requireAdministrator' uiAccess='false'"
    );
}

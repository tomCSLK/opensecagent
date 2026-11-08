//! The access list of the agent's folder `%ProgramData%\opensecagent`: full access for SYSTEM and the
//! administrators, nothing for anyone else, not inherited from `ProgramData` (where every user may read, and create
//! folders and files). The files in it inherit the list.
//!
//! Every user may create a folder in `ProgramData`, so a folder of that name may already be there before the agent
//! is: one that belongs to someone other than SYSTEM or the administrators, or that is a link, is refused instead of
//! used (its owner could change the access list again, a link could lead the service's writes elsewhere).

use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::Path;

use ::windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_INVALID_OWNER, HLOCAL, LocalFree, WIN32_ERROR,
};
use ::windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW, SDDL_REVISION_1,
    SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use ::windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, IsWellKnownSid,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use ::windows::Win32::Storage::FileSystem::{CreateDirectoryW, FILE_ATTRIBUTE_REPARSE_POINT};
use ::windows::core::{BOOL, HSTRING};

/// Owner the administrators, full access (also for folders and files in it) for SYSTEM and the administrators,
/// protected against inheritance.
const SDDL: &str = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

/// Creates the folder with the access list, or sets the list again on an existing folder of SYSTEM or the
/// administrators. Needs administrator rights.
pub fn protect(dir: &Path) -> io::Result<()> {
    let sd = SecurityDescriptor::from_sddl(SDDL)?;
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path = HSTRING::from(dir);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0.0,
        bInheritHandle: BOOL(0),
    };
    // SAFETY: the path is a valid wide string, the descriptor lives until the end of the function.
    match unsafe { CreateDirectoryW(&path, Some(&attributes)) } {
        Ok(()) => return Ok(()),
        Err(e) if e.code() == ERROR_ALREADY_EXISTS.to_hresult() => {}
        // Without administrator rights the administrators cannot be made the owner.
        Err(e) if e.code() == ERROR_INVALID_OWNER.to_hresult() => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "{} cannot be created without administrator rights",
                    dir.display()
                ),
            ));
        }
        Err(e) => return Err(io::Error::from(e)),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 || !meta.is_dir() {
        return Err(refused(dir, "is a link or not a folder"));
    }
    if !owned_by_system(&path)? {
        return Err(refused(
            dir,
            "belongs neither to SYSTEM nor to the administrators (made by another user?); remove it",
        ));
    }
    let dacl = sd.dacl()?;
    // SAFETY: path and access list are valid; the list lives in `sd` until the end of the function.
    win32(unsafe {
        SetNamedSecurityInfoW(
            &path,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl),
            None,
        )
    })
}

/// Whether the owner of the file or folder is SYSTEM or the administrators.
fn owned_by_system(path: &HSTRING) -> io::Result<bool> {
    let mut owner = PSID::default();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: the out pointers are valid; the descriptor is freed by `SecurityDescriptor`, `owner` points into it.
    win32(unsafe {
        GetNamedSecurityInfoW(
            path,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            &mut sd,
        )
    })?;
    let _sd = SecurityDescriptor(sd);
    Ok(is_system_or_administrators(owner))
}

fn is_system_or_administrators(sid: PSID) -> bool {
    // SAFETY: `sid` is a valid SID.
    unsafe {
        IsWellKnownSid(sid, WinLocalSystemSid).as_bool()
            || IsWellKnownSid(sid, WinBuiltinAdministratorsSid).as_bool()
    }
}

/// A security descriptor allocated by Windows, freed on drop.
struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl SecurityDescriptor {
    fn from_sddl(sddl: &str) -> io::Result<Self> {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: valid wide string and out pointer.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &HSTRING::from(sddl),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
        }?;
        Ok(Self(sd))
    }

    /// The access list; it lives as long as the descriptor.
    fn dacl(&self) -> io::Result<*const ACL> {
        let mut present = BOOL(0);
        let mut defaulted = BOOL(0);
        let mut dacl: *mut ACL = std::ptr::null_mut();
        // SAFETY: the descriptor is valid, the out pointers too.
        unsafe { GetSecurityDescriptorDacl(self.0, &mut present, &mut dacl, &mut defaulted) }?;
        if !present.as_bool() || dacl.is_null() {
            return Err(io::Error::other(
                "the security descriptor has no access list",
            ));
        }
        Ok(dacl)
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.0.is_null() {
            // SAFETY: allocated by Windows with LocalAlloc, freed once.
            unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
        }
    }
}

fn win32(e: WIN32_ERROR) -> io::Result<()> {
    if e.is_ok() {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(e.0 as i32))
    }
}

fn refused(dir: &Path, why: &str) -> io::Error {
    io::Error::other(format!("the folder {} {why}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::windows::Win32::Security::Authorization::ConvertStringSidToSidW;

    fn sid(text: &str) -> (PSID, impl Drop) {
        struct Free(PSID);
        impl Drop for Free {
            fn drop(&mut self) {
                // SAFETY: allocated by ConvertStringSidToSidW.
                unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
            }
        }
        let mut sid = PSID::default();
        // SAFETY: valid wide string and out pointer.
        unsafe { ConvertStringSidToSidW(&HSTRING::from(text), &mut sid) }.unwrap();
        (sid, Free(sid))
    }

    #[test]
    fn only_system_and_the_administrators_may_own_the_folder() {
        for (text, allowed) in [
            ("S-1-5-18", true),     // SYSTEM
            ("S-1-5-32-544", true), // Administrators
            ("S-1-5-32-545", false),
            ("S-1-1-0", false),
            ("S-1-5-21-1-2-3-1001", false),
        ] {
            let (sid, _free) = sid(text);
            assert_eq!(is_system_or_administrators(sid), allowed, "{text}");
        }
    }

    #[test]
    fn the_access_list_is_valid() {
        let sd = SecurityDescriptor::from_sddl(SDDL).unwrap();
        let dacl = sd.dacl().unwrap();
        // SAFETY: valid access list from the descriptor.
        assert_eq!(unsafe { (*dacl).AceCount }, 2);
    }

    #[test]
    fn a_link_or_a_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "x").unwrap();
        let e = protect(&file).unwrap_err();
        assert!(e.to_string().contains("not a folder"), "{e}");
        // A junction can be made without special rights.
        let target = dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let link = dir.path().join("link");
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .unwrap();
        assert!(made.status.success());
        let e = protect(&link).unwrap_err();
        assert!(e.to_string().contains("is a link"), "{e}");
    }

    /// With administrator rights (as in the CI) the folder is made with the list; without, Windows refuses the
    /// administrators as owner and nothing is made.
    #[test]
    fn a_new_folder_gets_the_access_list() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        match protect(&agent) {
            Ok(()) => {
                let path = HSTRING::from(agent.as_path());
                assert!(owned_by_system(&path).unwrap());
                // Again on the existing folder: the list is set again.
                protect(&agent).unwrap();
            }
            Err(e) => {
                assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
                assert!(!agent.exists());
            }
        }
    }
}

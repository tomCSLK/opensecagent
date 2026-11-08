# Changelog

All notable changes to this project will be documented in this file.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Security policy (`SECURITY.md`) and contribution guide (`CONTRIBUTING.md`).
- Project plan, license (AGPL-3.0-or-later), CI (Linux and Windows, cargo deny, list of third-party licenses).
- Command line `enroll`, `report`, `status`, `inventory`, `uninstall`, `version`.
- Enrollment with the enrollment code of opensecmon (`osa1_…`); the certificate is accepted by the fingerprint in
  the code or when it is valid for the address in the system's trust store.
- Reports as JSON with gzip, only the hash when nothing changed; a revoked computer stops reporting.
- Inventory of Debian, Ubuntu and systems based on them: os-release, running kernel, pending reboot, Ubuntu Pro,
  dpkg packages.
- Snaps, and Flatpaks of the system and of all users (read from the installation folders).
- Service mode `run`: reports at the interval opensecmon names with some jitter, backs off after failures, stops
  on SIGTERM.
- Enrollment dialog for the start menu (`enroll-dialog`, zenity or kdialog, German and English) that takes the
  code from the clipboard and enrolls through pkexec; OK with an empty field looks in the clipboard again, for a
  code copied after the dialog opened.
- The enrollment removes the agent script opensecmon offered before.
- A report waits if the Flatpaks of a user cannot be read, instead of leaving them out (which would remove them
  in opensecmon); homes on network file systems are named in the journal and left out.
- `status` also shows an error from before sending (the inventory could not be read) as the last error.
- Debian package files: hardened systemd service, polkit action, start menu entry and icon (`dev/deb.sh`).
- Inventory of Windows: the version of Windows, the programs of the machine and of the users signed in (Uninstall
  entries), Pythons of the Python install manager (PEP 514) and the store apps of all users.
- Windows installer (Inno Setup, `dev/build-installer.ps1`), signed with Authenticode. Icon, version information and
  manifest in the programs, the C runtime is linked in statically.
- Windows service: `run` works under the service manager (LocalSystem, delayed automatic start, restarted after
  failures) and writes its lines to `opensecagent.log` in the agent's folder; hidden commands `service-install` and
  `service-uninstall` for the installer. The agent's folder `%ProgramData%\opensecagent` gets an access list for
  SYSTEM and the administrators only; a folder of that name made by another user, or a link, is refused.
- Windows enrollment dialog (`enroll-dialog`, started by `opensecagent-enroll.exe` from the start menu entry
  "Enroll with opensecmon", which asks for administrator rights): takes the code from the clipboard, enrolls with a
  progress bar and says what went wrong, in German or English (texts shared with the Linux dialog).
- The installer sets up and starts the service, opens the enrollment dialog when the computer is not enrolled yet,
  adds the start menu entry, and enrolls without questions with `/VERYSILENT /CODE=…`; uninstalling removes the
  service and the enrollment. `enroll --code-file` reads the code from a file and deletes it.
- On Windows, the enrollment removes the agent script opensecmon offered before (scheduled task and folder
  `opensecmon-agent`).

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

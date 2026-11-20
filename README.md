# opensecagent

A small agent for Windows and Linux desktops that reports the installed software to
[opensecmon](https://github.com/tomCSLK/opensecmon), so its assets stay up to date without manual work.

**Status:** early development, **no release yet**. It works on Debian, Ubuntu and systems based on them, and on
Windows 10 and 11 (x64): it runs as a service, reports about every 15 minutes and is enrolled from a small dialog.
Installing updates from opensecmon is still to come. It replaces the two scripts opensecmon offered before
(`contrib/opensecmon-agent.sh`, `contrib/opensecmon-agent.ps1`); enrolling removes them.

Contributions are welcome, see [`CONTRIBUTING.md`](CONTRIBUTING.md); security issues please report as described in
[`SECURITY.md`](SECURITY.md).

## What it reports

- **Linux:** Debian packages, snaps and Flatpaks (of the system and of every user), the running kernel, whether a
  reboot is pending, and Ubuntu Pro.
- **Windows:** the version of Windows, the installed programs of the machine and of every user (also of users who are
  not signed in), Pythons from the Python install manager, and the store apps of all users.

## Using it

In opensecmon, add the computer under Assets → Computers (agent) → Add computer. The page offers the package for the
computer and the enrollment code. For a computer that already reports with the earlier script, use "Make a new code"
on its page instead: it then stays the same entry with its assets and triage.

### Debian and Ubuntu

1. Install the package `opensecagent_<version>_amd64.deb`.
2. Copy the enrollment code in opensecmon.
3. Open "Enroll with opensecmon" from the start menu. It finds the code in the clipboard (or lets you paste it)
   and asks for an administrator password once.

In a terminal, `sudo opensecagent enroll` does the same as the dialog, and `sudo opensecagent status` shows the
state. The service writes to the journal (`journalctl -u opensecagent`). Removing the package with
`apt purge opensecagent` also removes the enrollment.

### Windows 10 and 11

1. Run the installer `opensecagent-<version>-x64-setup.exe` (it asks for administrator rights). The installer comes
   with opensecmon after version 0.3.5.
2. Copy the enrollment code in opensecmon. At the end of the setup, the enrollment dialog takes the code from the
   clipboard; it also notices a code copied while it is open.

If you close the dialog, enroll later with "Enroll with opensecmon" from the start menu. Without questions:
`opensecagent-<version>-x64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /CODE=osa1_…`. The service writes its log to
`%ProgramData%\opensecagent\opensecagent.log`. Uninstalling (Settings → Apps) removes the service and the enrollment.

On both systems the computer stays in opensecmon until it is deleted there.

## How it works

- The agent connects to opensecmon; opensecmon never connects to the computer. No port has to be open on the
  computer.
- A computer is enrolled once with an enrollment code copied from opensecmon (`osa1_…`: address, certificate
  fingerprint and one-time code in one string). It then reports with its own token, over HTTPS, and accepts the
  certificate with that fingerprint, or one that is valid for the address in the usual way (opensecmon behind a
  proxy).
- Each report carries the full list of installed software. An unchanged list is sent as its hash only.
- Updates (planned) are installed only when the user asks for them in opensecmon. The packages come with opensecmon
  and are signed by the publisher; the agent checks the signature before installing.
- The interface is defined by opensecmon in `docs/agent-api.md`.
- On Linux the service runs as root with only the right to read files (needed for the Flatpaks in the users' home
  folders) and writes only `/var/lib/opensecagent`; the token there is readable for root only.
- On Windows the service runs as LocalSystem. Its folder `%ProgramData%\opensecagent` is open to SYSTEM and the
  administrators only. The programs of users who are not signed in are read from their `NTUSER.DAT` as a file,
  without loading it into the registry, so their next sign-in is not disturbed.
- The Windows programs are signed with Authenticode.

## License

Copyright (C) 2026 Thomas Cieslik

This program is free software: you can redistribute it and/or modify it under the terms of the GNU Affero General Public
License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.
See [`LICENSE`](LICENSE).

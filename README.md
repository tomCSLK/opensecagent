# opensecagent

A small agent for Windows and Linux desktops that reports the installed software to
[opensecmon](https://github.com/tomCSLK/opensecmon), so its assets stay up to date without manual work.

**Status:** early development, **no release yet**. On Debian and Ubuntu it works: it reports Debian packages,
snaps and Flatpaks (also those of single users), runs as a service and has an enrollment dialog. Windows is not
supported yet, and installing updates is still to come. Until then, opensecmon ships two scripts that do the same
job (`contrib/opensecmon-agent.sh` for Debian/Ubuntu, `contrib/opensecmon-agent.ps1` for Windows).

Contributions are welcome, see [`CONTRIBUTING.md`](CONTRIBUTING.md); security issues please report as described in
[`SECURITY.md`](SECURITY.md).

## Using it (Debian and Ubuntu)

1. Install the package `opensecagent_<version>_amd64.deb` (offered on the computer's page in opensecmon).
2. In opensecmon, add the computer under Assets → Computers (agent) → Add computer and copy the enrollment code.
3. Open "Enroll with opensecmon" from the start menu. It finds the code in the clipboard (or lets you paste it)
   and asks for an administrator password once.

That is all: the service reports about every 15 minutes. If the earlier script `opensecmon-agent` is installed,
the enrollment removes it. In a terminal, `sudo opensecagent enroll` does the same as the dialog, and
`sudo opensecagent status` shows the state. Removing the package with `apt purge opensecagent` also removes the
enrollment; the computer stays in opensecmon until it is deleted there.

## How it works

- The agent connects to opensecmon; opensecmon never connects to the computer. No port has to be open on the
  computer.
- A computer is enrolled once with an enrollment code copied from opensecmon (`osa1_…`: address, certificate
  fingerprint and one-time code in one string). It then reports with its own token, over HTTPS, and accepts the
  certificate with that fingerprint, or one that is valid for the address in the usual way (opensecmon behind a
  proxy).
- Updates (planned) are installed only when the user asks for them in opensecmon. The packages come with opensecmon
  and are signed by the publisher; the agent checks the signature before installing.
- Each report carries the full list of installed software (Debian packages, snaps, Flatpaks; later Windows
  programs and store apps). An unchanged list is sent as its hash only.
- The interface is defined by opensecmon in `docs/agent-api.md`.
- On Linux the service runs as root with only the right to read files (needed for the Flatpaks in the users' home
  folders) and writes only `/var/lib/opensecagent`; the token there is readable for root only.

## License

Copyright (C) 2026 Thomas Cieslik

This program is free software: you can redistribute it and/or modify it under the terms of the GNU Affero General Public
License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.
See [`LICENSE`](LICENSE).

# Security policy

## Supported versions

Security fixes go into the next release; only the latest release is supported.

Dependencies are checked against the RustSec advisory database (`cargo deny`, rules in `deny.toml`) on every change and every
week.

## Reporting a vulnerability

Please report vulnerabilities **privately** and not as a public issue:

1. Open the [Security tab](https://github.com/tomCSLK/opensecagent/security) of the repository.
2. Choose **Report a vulnerability** and describe the problem.

Helpful details: the affected version, the operating system, how to reproduce it, what an attacker can achieve and under which
conditions (for example: another local user, a crafted Flatpak or snap, a server that pretends to be opensecmon). Do not include
tokens, enrollment codes or inventories of your own computers.

opensecagent is maintained in spare time. You will get an answer as soon as possible; once a fix is released, the advisory is
published and you are credited if you want.

A vulnerability in the server side of the agent API belongs to [opensecmon](https://github.com/tomCSLK/opensecmon/security).

## Scope

In scope: the service and the command line, the enrollment dialog and its use of `pkexec`, the reading of the inventory (dpkg,
snap, Flatpak, later the Windows registry), the HTTPS connection to opensecmon and the check of its certificate, the storage of
the token, the removal of the earlier script and the Debian package with its systemd unit and polkit action.

Known and documented, not vulnerabilities in themselves:

- On Linux the service runs as root, limited to reading files (`CAP_DAC_READ_SEARCH`, needed for the Flatpaks in the users' home
  folders) and writing only `/var/lib/opensecagent`. The token there is readable for root only.
- The enrollment code contains the address of opensecmon, the fingerprint of its certificate and a one-time code. Whoever gets the
  code before it is used can enroll a computer of their own in its place; opensecmon shows every enrollment.
- The agent reports the names and versions of the installed software. It does not read file contents, and it never runs commands
  it receives from opensecmon.
- Installing updates is not implemented yet. When it is, packages will only be installed after the user asks for it in
  opensecmon and after the signature of the publisher (`keys/allowed_signers`) has been checked.

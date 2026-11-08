# Contributing to opensecagent

Thanks for your interest! opensecagent reports the installed software of a desktop computer to
[opensecmon](https://github.com/tomCSLK/opensecmon). Trying it on your distribution and telling us what is missing or reported
wrongly is a valuable contribution.

## Reporting bugs and ideas

- **Bugs:** open an issue. Include the version, your operating system and desktop, and the output of `sudo opensecagent status`.
  For wrong or missing software, `sudo opensecagent inventory` shows what would be reported; quote only the lines that matter.
  Never post tokens or enrollment codes.
- **Ideas:** open an issue and describe what you want to achieve.
- **Security vulnerabilities:** please do **not** open a public issue, see [`SECURITY.md`](SECURITY.md).

For a larger change, open an issue first so we can agree on the approach. Anything that changes what is sent to opensecmon is first
defined in opensecmon's `docs/agent-api.md`.

## Development

Requirements: a current stable Rust toolchain (edition 2024). Before you open a pull request, all of these must pass:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
cargo deny --locked check
```

The tests need no network and no opensecmon: they use a small HTTPS stand-in. CI runs the same checks on Linux and Windows for every
pull request; `cargo deny` also runs weekly. The Debian package is built with `dev/deb.sh` (needs `cargo-deb`).

## Conventions

- **Language:** code, identifiers, code comments, commit messages, the README and the changelog are English; only the internal
  planning files `PLAN.md` and `CLAUDE.md` are German.
- **Users first:** people who install the agent should need neither a terminal nor keys, fingerprints or configuration files.
- **Output of other programs** (`dpkg-query`, `snap`, `flatpak`, the registry) is data: check its size and form, never run it.
- **Dependencies:** only maintained crates under a license compatible with the AGPL (see `deny.toml`). Give a short reason in the
  commit message. After any change to `Cargo.lock`, run `python3 -I dev/third-party-licenses.py` and commit
  `THIRD-PARTY-LICENSES.txt`.
- Small, self-contained commits; every commit should build and pass the tests.

## Use of AI tools

Large parts of opensecagent were written with the help of an AI assistant (Claude). If you use AI tools for a contribution, say so
in the pull request, review the result yourself and make sure you can explain every change.

## Developer Certificate of Origin (DCO)

opensecagent uses the [Developer Certificate of Origin 1.1](https://developercertificate.org/) instead of a contributor license
agreement. By signing off a commit you certify that you wrote the change, or otherwise have the right to submit it under the
project's license. `git commit -s` adds the line `Signed-off-by: Your Name <you@example.org>`; CI checks every commit of a pull
request.

## License

opensecagent is licensed under the [GNU Affero General Public License v3.0 or later](LICENSE). Any contribution you submit is
licensed the same way.

# Windows-Fassung von opensecagent: Weiterarbeit auf einem Windows-Rechner

Stand: 2026-11-08. Diese Datei beschreibt, was nötig ist, um opensecagent auf einem Windows-Rechner weiterzuentwickeln
und dort die Windows-Fassung (Phase 3 in `PLAN.md`) zu bauen, zu testen und als Installer zu verpacken. Was im Code zu
tun ist, steht in `PLAN.md` im Abschnitt „Phase 3: Windows (Plan)“; diese Datei ist die Anleitung für den Rechner.

**Heutiger Stand:** Unter Windows bauen das Programm und die Tests schon (CI-Job `windows-2025`, clippy und
`cargo test` grün). Es kann sich anmelden und melden, aber das Inventar sagt unter Windows nur „the Windows inventory
comes with a later version“; Dienst, Installer und Anmeldung über den Installer fehlen. Bis dahin melden sich
Windows-Rechner mit dem PowerShell-Skript aus opensecmon (`contrib/opensecmon-agent.ps1`), das zugleich die Vorlage
für das Inventar ist.

## 1. Was auf dem Windows-Rechner installiert sein muss

Windows 10/11 x64. Die meisten Werkzeuge sind dieselben wie für die Windows-Fassung von openpacketmon; Npcap
braucht opensecagent nicht.

| Was | Wofür | Installation |
|---|---|---|
| Git für Windows | Repo, Git Bash für die `dev/*.sh`-Skripte | `winget install Git.Git` |
| Visual Studio Build Tools mit „Desktop development with C++“ und Windows SDK | Linker, `rc.exe` für Icon und Versionsangaben | `winget install Microsoft.VisualStudio.2022.BuildTools`, dann die Komponente im Visual Studio Installer wählen; ein vorhandenes Visual Studio mit C++-Werkzeugen und SDK genügt |
| Rust (rustup, Toolchain `stable-x86_64-pc-windows-msvc`) | Bauen | `winget install Rustlang.Rustup`; `%USERPROFILE%\.cargo\bin` steht erst in einem **neuen** Terminal im Pfad |
| Python 3 | `dev/third-party-licenses.py` | `winget install Python.Python.3.12` |
| GitHub CLI | Anmeldung, Push, CI ansehen | `winget install GitHub.cli` |
| cargo-deny 0.20.2 | Lizenz- und Sicherheitsprüfung (gleiche Version wie die CI) | `cargo install cargo-deny --locked --version 0.20.2` |
| Inno Setup 6 | Installer (Phase 3, Schritt 3d) | `winget install JRSoftware.InnoSetup --scope user` |
| WSL 2 mit Ubuntu (optional) | Wegwerf-Instanz von opensecmon zum Testen (Abschnitt 5) | `wsl --install -d Ubuntu-24.04` (Administrator, danach Neustart) |
| Claude Code | Weiterarbeit | wie gewohnt |

Zum Ausprobieren des Installers ist eine zweite, saubere Windows-Umgebung sinnvoll (virtuelle Maschine oder die
Windows-Sandbox), in der noch nichts von opensecagent und kein Rust installiert ist.

## 2. Repos holen

opensecagent verweist in `CLAUDE.md` und `PLAN.md` auf opensecmon unter `../SecurityDashboad/` (so heißt der Ordner
auf dem Linux-Laptop). Damit die Verweise stimmen, beide Repos nebeneinander holen und opensecmon in diesen Ordner:

```powershell
gh auth login                      # Konto tomCSLK; beide Repos sind öffentlich, zum Pushen braucht es die Anmeldung
mkdir $env:USERPROFILE\Projekte; cd $env:USERPROFILE\Projekte
gh repo clone tomCSLK/opensecagent
gh repo clone tomCSLK/opensecmon SecurityDashboad
cd opensecagent
git config user.name tomCSLK
git config user.email m.tomcs@gmail.com
```

`.gitattributes` setzt für Textdateien `eol=lf`; die Zeilenenden bleiben also auch unter Windows richtig, und die
Shell-Skripte in `dev/` laufen danach wieder unter Linux.

Aus opensecmon wird unter Windows nur gelesen (`docs/agent-api.md`, `contrib/opensecmon-agent.ps1`, `src/agent.rs`,
`src/inventory/windows.rs`, `src/inventory/items.rs`). Braucht die Schnittstelle etwas Neues, wird es zuerst dort
festgelegt (Regel in `CLAUDE.md`); das geht auch vom Windows-Rechner, dann gelten die Regeln von
`../SecurityDashboad/CLAUDE.md` (die Tests von opensecmon laufen nur unter Linux, also in WSL oder auf dem Laptop).

## 3. Dateien, die nicht im Repo sind

- **`CLAUDE.local.md`** (Arbeitsregeln und Angaben zu den eigenen Rechnern) liegt nur auf dem Linux-Laptop. Eine
  Fassung für den Windows-Rechner bringt der Nutzer selbst mit und legt sie in den Ordner `opensecagent\` (sie ist
  gitignored). Ohne sie fehlen Claude Code Regeln, die nicht öffentlich stehen sollen.
- **Signaturschlüssel des Herausgebers:** bleibt auf dem Linux-Laptop. Signiert wird dort (Abschnitt 6).
- **`dist/`** (gebaute und signierte Pakete): nur auf dem Laptop, unter Windows nicht nötig.
- Notizen von Claude Code und die Übersicht über alle Projekte liegen nur auf dem Laptop; die nötigen Regeln stehen in
  `CLAUDE.md` bzw. in der mitgebrachten `CLAUDE.local.md`.

## 4. Erster Lauf

In einem neuen PowerShell-Fenster im Ordner `opensecagent`:

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo deny --locked check
python -I dev/third-party-licenses.py --check
```

Alle fünf müssen grün sein, bevor die Arbeit beginnt (die CI prüft dasselbe unter Linux und Windows).

**Smart App Control** (falls an) blockiert ab und zu frisch gebaute, unsignierte Dateien in `target\`, auch Tests und
Build-Skripte von Abhängigkeiten („Eine Anwendungssteuerungsrichtlinie hat diese Datei blockiert“, Fehler 4551).
Abhilfe aus openpacketmon: den betroffenen Ordner unter `target\…\build\` löschen oder neu linken (Zeitstempel von
`src/main.rs` ändern), dann noch einmal bauen. Seit 2026-11-08 blockiert es hier auch `cargo fmt` (`cargo-fmt.exe` der
Toolchain ist unsigniert) und das selbst gebaute `cargo-deny.exe`. Ersatz für `cargo fmt --check`:
`& (rustup which rustfmt) --check --edition 2024 build.rs src\main.rs` (prüft das ganze Crate); `cargo deny` prüft die
CI bei jedem Push, oder in WSL.

Die Linux-Fassung muss unverändert weiter bauen: Windows-Code hinter `#[cfg(windows)]`, Linux-Code hinter
`#[cfg(target_os = "linux")]`. Die CI prüft beides bei jedem Push. Linux-Tests lassen sich unter Windows nicht
ausführen; wer Linux-Code anfasst, prüft in WSL (`cargo test` dort im selben Ordner unter `/mnt/c/…` ist langsam, besser
ein eigener Klon in WSL) oder wartet auf die CI.

## 5. Ausprobieren

**Ohne Administrator und ohne Netz:** `cargo run -- --dir $env:TEMP\osa inventory` zeigt, was gemeldet würde (sobald
das Inventar da ist). Programme anderer Benutzer und Store-Apps aller Benutzer brauchen Administrator- bzw.
SYSTEM-Rechte; als SYSTEM testen z. B. mit `psexec -s` (Sysinternals) oder über den Dienst.

**Gegen opensecmon nur mit einer Wegwerf-Instanz, nie gegen ein produktives opensecmon.** Am einfachsten in WSL mit
dem Programm aus dem Release (statisch gebaut, läuft in jeder Linux-Umgebung; das Paket enthält auch opensecagent
0.1.2 mit Signatur, damit zeigt die Seite des Rechners einen Anmeldecode):

```bash
# in WSL (Ubuntu), in einem eigenen Ordner
mkdir -p ~/osm-test && cd ~/osm-test
curl -LO https://github.com/tomCSLK/opensecmon/releases/download/v0.3.5/opensecmon_0.3.5-1_amd64.deb
curl -LO https://github.com/tomCSLK/opensecmon/releases/download/v0.3.5/SHA256SUMS
sha256sum -c SHA256SUMS && dpkg-deb -x opensecmon_0.3.5-1_amd64.deb pkg
cat > test.toml <<'EOF'
listen = "127.0.0.1:8445"
tls_cert_file = "tls-cert.pem"
tls_key_file = "tls-key.pem"
allowed_hosts = ["127.0.0.1:8445"]
database_path = "test.db"
secret_key_file = "secret.key"
cookie_secure = true
fetch_sources = false
agent_packages_dir = "pkg/usr/share/opensecmon/agent"
EOF
B=pkg/usr/bin/opensecmon
$B --config test.toml key generate
$B --config test.toml tls generate --name 127.0.0.1
$B --config test.toml user add test        # fragt nach einem Passwort
$B --config test.toml serve
```

Unter Windows dann `https://127.0.0.1:8445` öffnen (WSL 2 reicht `127.0.0.1` an Windows weiter), anmelden, unter
Assets → Rechner (Agent) einen Rechner anlegen, den Anmeldecode kopieren und mit
`cargo run -- --dir $env:TEMP\osa enroll` (Code auf der Eingabe) anmelden. Ohne Quellen gibt es keine Treffer, aber
Anmeldung, Meldung, Kurzform, `409`, `429` und die Seite „Nicht zugeordnet“ lassen sich prüfen. Danach den Ordner
`~/osm-test` löschen. Die Instanz nie mit `fetch_sources = true` starten (Erstimport rund 1,9 GB).

**Echter Test** auf dem eigenen Windows-Rechner gegen das eigene opensecmon: Installation und Anmeldung macht der
Nutzer (Arbeitsregel in `CLAUDE.md`), Claude liest nur die Ergebnisse.

## 6. Installer bauen und signieren (Herausgeber)

Ablauf (entschieden 2026-11-07, `PLAN.md` „Entschieden“ Punkte 6 und 8):

1. Auf dem Windows-Rechner bauen und mit Authenticode signieren, in einem Schritt (nur auf Auftrag):
   `powershell -ExecutionPolicy Bypass -File dev\build-installer.ps1 -CertSha1 <Fingerabdruck>` (oder Variable
   `OPENSECAGENT_SIGN_SHA1`). Ergebnis `target\installer\opensecagent-<version>-x64-setup.exe`, der Name, den
   opensecmon erwartet (`docs/agent-api.md`, „Updates of opensecagent“). Das Skript signiert `opensecagent.exe` vor dem
   Verpacken; Inno Setup signiert Installer, Deinstaller und die temporären Kopien, die Setup beim Start entpackt
   (`SignTool`, `SignedUninstaller`). Alle mit SHA-256 und Zeitstempel von Certum (`http://time.certum.pl`), damit die
   Signatur auch nach Ablauf des Zertifikats gültig bleibt; danach `signtool verify /pa /all`. Die Authenticode-Signatur
   muss **vor** der eigenen Signatur kommen, weil sie die Datei verändert.
   - Zertifikat: Certum „Open Source Code Signing in the Cloud“ (seit 2026-11 ausgestellt, ein Jahr gültig).
     SimplySign Desktop muss verbunden sein (Anmeldung mit einem Code aus der SimplySign-App), dann liegt das
     Zertifikat im Speicher des Benutzers. Fingerabdruck:
     `Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert | Format-List Subject, Thumbprint, NotAfter`.
   - `signtool.exe` kommt mit dem Windows SDK; das Skript sucht es selbst, `PATH` ist nicht nötig.
   - Blockiert Smart App Control beim Bauen eine Datei unter `target\` (Abschnitt 4), den genannten Ordner löschen und
     das Skript noch einmal starten.
3. Die fertige `setup.exe` auf den Linux-Laptop holen (z. B. als Anhang eines Entwurfs-Releases, über das NAS oder
   einen USB-Stick) und dort vom Nutzer mit dem Herausgeber-Schlüssel signieren:
   `ssh-keygen -Y sign -f <Schlüssel> -n opensecagent opensecagent-<version>-x64-setup.exe` (ergibt `.sig`).
4. Auf dem Laptop in opensecmon `dev/agent-packages.sh <Ordner>` (prüft Namen und Signatur gegen
   `keys/allowed_signers`), dann das opensecmon-Paket bauen. Ein neuer Agent kommt so mit dem nächsten
   opensecmon-Paket zu den Anwendern.

Ohne Authenticode-Signatur warnt SmartScreen, und Smart App Control kann den Installer ganz blockieren; für eigene
Tests reicht die unsignierte Fassung (Smart App Control aus oder Testrechner ohne). Mit Signatur kann SmartScreen
anfangs trotzdem warnen, bis der Installer genug Reputation hat; Smart App Control lässt Dateien mit gültiger
Signatur einer vertrauenswürdigen Zertifizierungsstelle laufen.

## 7. Zurück auf den Linux-Laptop

Commits wie gewohnt pushen (klein, Englisch, `git commit -s`). Auf dem Laptop danach `git pull` in `opensecagent`;
der Stand steht in `PLAN.md`. Die Zusammenfassung in der Projektübersicht auf dem Laptop wird dort nachgezogen.

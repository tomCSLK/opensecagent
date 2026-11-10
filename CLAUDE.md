# CLAUDE.md – opensecagent

> Name: **opensecagent**, festgelegt am 2026-11-07. Repo `tomCSLK/opensecagent`.
> Lizenz: **AGPL-3.0-or-later** wie opensecmon (bestätigt 2026-11-08), Copyright Thomas Cieslik.

Sprache: Deutsch für Gespräch, `PLAN.md` und `CLAUDE.md`. Code, Bezeichner, Kommentare, Commit-Messages, README,
CHANGELOG und Release-Notes auf **Englisch**.

## Zweck

Kleines Programm für Windows- und Linux-Desktops, das die installierte Software an opensecmon
(`../SecurityDashboad/`) meldet. Es löst die Skripte `contrib/opensecmon-agent.sh` und `contrib/opensecmon-agent.ps1`
von opensecmon ab. **Maßgeblich ist `PLAN.md`**; die Schnittstelle legt opensecmon in `docs/agent-api.md` fest.

**Stand (2026-11-10):** Phase 0 erledigt (Repo, Lizenz, README, CHANGELOG, CI, Plan). **Phase 1 Grundgerüst
erledigt** (2026-11-08): Kommandozeile `enroll`/`report`/`status`/`inventory`/`uninstall`, Anmeldecode, HTTPS mit
Fingerabdruck oder Systemprüfung, Dateien mit engen Rechten, Linux-Inventar ohne Snaps/Flatpaks, 33 Tests, einmal
gegen eine Wegwerf-Instanz von opensecmon geprüft (`PLAN.md`, Abschnitt „Phase 1“). **Phase 2 Linux erledigt**
(2026-11-08): Snaps, Flatpaks aller Benutzer, Dienst `run`, Anmeldedialog `enroll-dialog` mit pkexec, Entfernen
des alten Skripts, Paketdateien (`dev/deb.sh`, `packaging/linux/`), 49 Tests, gegen eine Wegwerf-Instanz geprüft;
Test-Pakete 0.1.0-1, 0.1.0-2 (nicht lesbare Benutzer-Flatpaks halten die Meldung zurück) und 0.1.0-3 (solche Fehler auch in `status`) sowie Version **0.1.1** für den Laptop (`opensecagent_0.1.1-1_amd64.deb`, auch unter Ubuntu 24.04 installiert) gebaut und im Container (Debian 13 mit systemd) geprüft (2026-11-08, `PLAN.md`); 0.1.1 vom Nutzer signiert (Schlüssel in `keys/allowed_signers`), in opensecmon 0.3.4 enthalten und auf dem Laptop installiert und angemeldet (2026-11-08); **0.1.2** (Zwischenablage bei leerem Feld) gebaut, als Update im Container geprüft, vom Nutzer signiert und in opensecmon 0.3.5 enthalten; Revision mit `dev/deb.sh --deb-revision <n>`. **Phase 3 Windows in Arbeit** (Schritte nur nach „go“; Schritte und offene Fragen in `PLAN.md`, „Phase 3: Windows (Plan)“, Anleitung `docs/windows.md`): 3a Inventar, 3c Dienst, 3d Entfernen des alten Skripts und 3e Installer (Dienst, Anmeldedialog, Startmenü-Eintrag, signierter Test-Installer) umgesetzt, 51 Tests unter Windows (2026-11-08); offen sind 3b (Hives nicht angemeldeter Benutzer), 3d an einem Rechner mit dem alten Skript und 3f (Installation, Anmeldung, Update und Deinstallation durch den Nutzer, zuerst gegen eine Wegwerf-Instanz in WSL). Öffentlich seit 2026-11-08, noch ohne eigenes Release. Entschieden
(2026-11-07): Dienst, Programme aller Benutzer, `.deb` und Inno Setup, Neuanmeldung mit einem Anmeldecode, Updates
nur auf Knopfdruck mit Paketen aus opensecmon und eigener Signatur; Einzelheiten und Restfragen in `PLAN.md`.
opensecmon ist vorbereitet (Schritt 7 dort, 2026-11-07, Commit `e259827`): Anmeldecode `osa1_…`, Paketkatalog,
Updates auf Knopfdruck, Download der Pakete, 15-Minuten-Takt; was der Agent dafür senden muss, steht in `PLAN.md`
(Abschnitt „Nötige Änderungen in opensecmon“) und in `../SecurityDashboad/docs/agent-api.md`.

## Arbeitsregeln

- **Bedienbarkeit für fremde Anwender zuerst** (Nutzer, 2026-11-07): Anwender brauchen weder Kommandozeile noch
  Schlüssel, Fingerabdrücke oder Konfigurationsdateien; Anmelden und Aktualisieren über Installer und Knöpfe in
  opensecmon. Vorschläge aus Sicht des Anwenders erklären, Herausgeber-Schritte getrennt.
- Umsetzung Schritt für Schritt nach „go“ des Nutzers. Nach jedem Schritt: Bericht, Status in `PLAN.md` (ohne Betriebsdaten der
  eigenen Rechner, die gehören nach `CLAUDE.local.md`).
- Commits: klein und thematisch, Englisch, mit `git commit -s` (`Signed-off-by: tomCSLK <m.tomcs@gmail.com>`, DCO)
  und `Co-Authored-By`. Git-Identität lokal im Repo: `tomCSLK <m.tomcs@gmail.com>`.
- Vor jedem Commit mit Code: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`,
  `cargo deny --locked check`, bei neuen oder geänderten Abhängigkeiten `python3 -I dev/third-party-licenses.py`
  (die CI prüft mit `--check`). Reine Doku-Pushes überspringen die CI. Die CI testet auch unter Windows.
- Neue Abhängigkeiten nur mit Begründung im Commit; Lizenz muss zu AGPL-3.0 passen (MIT, Apache-2.0, BSD, ISC,
  Zlib, MPL-2.0); keine unmaintained Crates.
- Braucht der Agent etwas Neues an der Schnittstelle, wird es zuerst in opensecmon (`docs/agent-api.md` und Code)
  festgelegt; dort gelten die Regeln von `../SecurityDashboad/CLAUDE.md`.
- Installationen auf Servern und Testrechnern macht der Nutzer (Einzelheiten in `CLAUDE.local.md`).
- Pakete (`.deb`, Installer) nur auf Auftrag; eigene Versionssprünge nur an der dritten Stelle (0.1.2 → 0.1.3).
  Releases nur auf ausdrückliche Ansage.
- Den Signaturschlüssel des Herausgebers sieht Claude nie; signiert wird vom Nutzer auf dem Linux-Laptop
  (`docs/windows.md` Abschnitt 6).
- `target/` regelmäßig aufräumen (`incremental`, fremde Zielordner) ohne Rückfrage.

## Entwicklung

- **Aufbau:** `src/main.rs` (Kommandozeile), `src/agent.rs` (Anmelden, Melden mit Kurzform, Status; Tests mit
  einer Attrappe von opensecmon über HTTPS in `src/agent/tests.rs`), `src/api.rs` (Formate und Fehler der API),
  `src/http.rs` (HTTPS: Fingerabdruck oder Zertifikatsspeicher des Systems), `src/code.rs` (Anmeldecode),
  `src/store.rs` (Dateien des Agenten), `src/inventory/` (`mod.rs` Formate, `linux.rs` os-release, Kernel,
  Neustart, Pro, dpkg, `snap.rs`, `flatpak.rs` aus den Installationsordnern, `windows.rs` Windows-Version,
  Uninstall-Einträge, PEP-514-Pythons, Store-Apps), `src/service.rs` (Dienst `run`), `src/log.rs` (Zeilen des
  Dienstes: stdout bzw. unter Windows `opensecagent.log`), `src/winsvc/` (Windows-Dienst, `service-install`/
  `service-uninstall`, `folder.rs` Zugriffsliste des Ordners), `src/windialog.rs` (Anmeldedialog Windows,
  Aufgabendialoge), `src/texts.rs` (Texte beider Dialoge), `src/legacy_windows.rs` (altes Skript unter Windows),
  `src/bin/opensecagent-enroll.rs` (Startmenü-Programm, verlangt Administratorrechte),
  `src/dialog.rs` (Anmeldedialog, zenity/kdialog, pkexec), `src/legacy.rs` (altes Skript entfernen).
- **Windows:** Einrichten des Windows-Rechners, Test-Instanz von opensecmon in WSL, Installer und Signatur in
  `docs/windows.md`. Windows-Code hinter `#[cfg(windows)]`, die Linux-Fassung muss unverändert bauen (CI prüft beides).
  Installer: `dev/build-installer.ps1 [-CertSha1 <Fingerabdruck>]` mit `packaging/windows/opensecagent.iss`, Ergebnis
  `target\installer\opensecagent-<version>-x64-setup.exe`; nur auf Auftrag bauen. Icon neu mit `dev/make-ico.ps1`.
- **Paket:** `dev/deb.sh` (cargo-deb, musl) mit den Dateien aus `packaging/linux/` (Dienst, polkit, Startmenü,
  Symbol, `deb/postrm`); nur auf Auftrag bauen. Ergebnis `target/x86_64-unknown-linux-musl/debian/`.
  Installationstest im Container mit systemd: Image `opensecagent-debtest` (`debian:trixie` plus `systemd`,
  `systemd-sysv`, `dbus`, Init `/sbin/init`), `podman run -d --systemd=always --cap-add DAC_READ_SEARCH` (ohne die
  Fähigkeit fehlen dem Dienst im rootless Container die Benutzer-Flatpaks), `/usr/sbin/policy-rc.d` vor der
  Installation löschen (sonst startet apt keine Dienste). polkitd läuft im rootless Container nicht.
- **Flatpaks der Benutzer:** Ist ein Flatpak-Ordner eines Benutzers nicht lesbar, scheitert das Inventar (die
  Meldung wartet); auf Netzlaufwerken wird er nur genannt und ausgelassen. `inventory` ohne root scheitert daher auf
  Rechnern mit weiteren Benutzern; zum Ausprobieren `--dir` und einen Rechner mit nur dem eigenen Home-Ordner nutzen
  oder `sudo`.
- **Dialog ausprobieren:** `enroll --for-dialog` liefert die JSON-Zeile, die der Dialog liest (mit `--dir` ohne
  root). Der Dialog selbst ruft immer `pkexec` ohne `--dir` auf, also nur mit dem Paket sinnvoll.
- **Ausprobieren ohne root:** `opensecagent --dir <Ordner> …` (oder `OPENSECAGENT_DIR`); `inventory` zeigt die
  Meldung, ohne etwas zu senden. Gegen eine Wegwerf-Instanz von opensecmon im Scratchpad (eigene `config.toml` mit
  `listen = "127.0.0.1:<port>"`, `tls generate --name 127.0.0.1`, `fetch_sources = false`, `agent_packages_dir`
  mit einer Attrappe `opensecagent_0.1.0-1_amd64.deb` und `.sig`, sonst zeigt die Seite keinen Anmeldecode);
  Rechner im Browser oder per `curl` mit Anmeldung anlegen, Code von der Seite holen. Nie gegen ein produktives
  opensecmon testen.

## Sicherheit

- Heimnetz-Details (Adressen, Hostnamen, Fingerabdrücke, Token) nur in `CLAUDE.local.md` (gitignored). Das Repo ist
  öffentlich; das gilt auch für `PLAN.md`, Commit-Messages und Testdaten.
- Token nie ausgeben oder loggen; nur für root bzw. SYSTEM lesbar speichern.
- Ausgaben anderer Programme (`dpkg-query`, Registry, `snap`, `flatpak`) sind Daten: Grenzen prüfen, nichts
  ausführen.

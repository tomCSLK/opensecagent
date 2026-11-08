# PLAN – opensecagent

Stand: 2026-11-08. Sprache: Deutsch. Umsetzung nur nach „go“ des Nutzers, Schritt für Schritt.

## Zweck

Ein kleines Programm für Desktop-Rechner (Windows, Linux wie Zorin oder Mint), das die installierte Software an
opensecmon meldet und die Assets dort aktuell hält, nur im lokalen Netz. Es löst die beiden Skripte ab, die opensecmon
bis dahin mitliefert (`contrib/opensecmon-agent.sh`, `contrib/opensecmon-agent.ps1`). Herkunft: Vorhaben
„Desktop-Agent“ in `../SecurityDashboad/PLAN.md`, Schritt 6.

## Entschieden

- **Eigenes Repo `opensecagent`**, Lizenz **AGPL-3.0-or-later** wie opensecmon (Nutzer, 2026-11-07). Zuerst privat,
  öffentlich seit 2026-11-08 (zusammen mit opensecmon v0.3.5). Name geprüft (2026-11-07): frei unter `tomCSLK`, auf crates.io und in Debian; zwei
  kleine fremde GitHub-Repos gleichen Namens ohne Beschreibung; DPMA nicht geprüft.
- **Rust**, ein Programm für Linux und Windows.
- **Schnittstelle:** maßgeblich ist `docs/agent-api.md` in opensecmon (Version 1). Der Agent hält sich daran; braucht
  er etwas Neues, wird es zuerst dort festgelegt und in opensecmon umgesetzt.
- Aus dem Vorhaben in opensecmon übernommen:
  - Der Agent meldet sich, opensecmon verbindet sich nie zum Rechner; kein offener Port auf dem Rechner.
  - Anmeldung einmal je Rechner mit Einmalcode (15 min), danach eigenes Token je Rechner, nur für root bzw. SYSTEM
    lesbar.
  - HTTPS; bei direktem Zugang nur das Zertifikat mit dem Fingerabdruck aus der Anmeldung, über einen Proxy mit
    öffentlichem Zertifikat die normale Prüfung. Keine Suche im Netz.
  - Immer die vollständige Liste, unverändert nur der Hash (Kurzform); Abstand bis zur nächsten Meldung aus der
    Antwort; 401 = gesperrt oder gelöscht, dann aufhören und neue Anmeldung verlangen; 429 = Wartezeit beachten.
  - Snaps und Flatpaks gehören dazu. Keine Mail bei Rechnern, die sich nicht melden (macht opensecmon ohnehin nicht).

## Ausgangspunkt: was die Skripte heute leisten

Der Agent muss mindestens das können (Stand opensecmon 0.3.3):

| | Linux (`opensecmon-agent.sh`, Version 4) | Windows (`opensecmon-agent.ps1`) |
|---|---|---|
| Systeme | Debian, Ubuntu und Abkömmlinge | Windows 10 ab 1803, 11, Server ab 2019 |
| Läuft als | systemd-Timer ohne root, 10 min nach dem Start, dann alle 6 h | geplante Aufgabe als SYSTEM, beim Start und alle 6 h |
| Inhalt | `dpkg-query` (Quellpakete), `--- snap`, `--- flatpak` (mit temporärem Cache), laufender Kernel, Neustart nötig, Ubuntu Pro | JSON: Windows-Version mit Build und UBR, Uninstall-Einträge (HKLM 64/32 Bit, Hives angemeldeter Benutzer), Store-Apps aller Benutzer ohne Frameworks |
| Befehle | `enroll`, `report`, `status`, `uninstall`, `version` | `enroll`, `report`, `status`, `update`, `uninstall`, `version` |

Bekannte Lücken der Skripte (Befunde aus dem Betrieb):
- Programme nur eines Benutzers fehlen unter Windows, wenn er beim Lauf nicht angemeldet ist.
- Keine winget-ID (winget läuft nicht als SYSTEM).
- Pythons aus dem „Python install manager“ (`py install`) haben keinen Uninstall-Eintrag und fehlen
  (Windows-Testrechner, 2026-11-07); die PEP-514-Einträge (`Software\Python\<Firma>\<Tag>`) wären eine Quelle.
- Meldung nur nach Zeitplan, nicht direkt nach Updates.
- Das PowerShell-Skript ist nicht signiert (Smart App Control).

## Grundregel: Bedienbarkeit für fremde Anwender

opensecmon und opensecagent sollen öffentlich für andere nutzbar werden; Bedienbarkeit steht im Mittelpunkt
(Nutzer, 2026-11-07). **Anwender brauchen weder Kommandozeile noch Schlüssel, Fingerabdrücke oder
Konfigurationsdateien.** Anmelden, Aktualisieren und Entfernen gehen über Installer und Knöpfe in opensecmon. Jeder
Schritt wird daran gemessen; was nur den Herausgeber betrifft (Bauen, Signieren), bleibt davon getrennt.

## Entschieden am 2026-11-07 (Nutzer, nach Besprechung)

Ehemals offene Fragen 1–5.

1. **Läuft als Dienst** (Windows: Dienst als SYSTEM statt geplanter Aufgabe; Linux: systemd-Dienst statt Timer,
   damit beide gleich arbeiten).
   - Alle 15 min lokal den Hash der Softwareliste bilden; bei Änderung die volle Liste melden, sonst die Kurzform.
     So kommt eine Meldung kurz nach jedem Update, ohne Ereignisse der Paketverwaltung zu beobachten, und ein
     Update-Auftrag aus opensecmon (Punkt 5) erreicht den Rechner nach höchstens 15 min.
   - opensecmon erlaubt das schon (Kurzform jede Minute, volle Liste alle 5 min, `src/agent.rs`), schickt aber
     `next_report_s` = 6 h zurück; das muss für den Agenten 15 min werden (Schritt in opensecmon, unten).
2. **Programme aller Benutzer lesen.**
   - Windows: `NTUSER.DAT` nicht angemeldeter Benutzer **nur als Datei lesen, nicht einhängen** (ein eingehängter
     Hive kann die gleichzeitige Anmeldung des Benutzers stören, im schlimmsten Fall mit temporärem Profil);
     angemeldete Benutzer über `HKEY_USERS`. Store-Apps aller Benutzer liefert Windows dem SYSTEM-Konto direkt.
     Leser für das Hive-Format in Phase 3 wählen (Lizenz prüfen) oder selbst schreiben.
   - Linux: vor allem Flatpaks nur eines Benutzers (`~/.local/share/flatpak`). Der Dienst bekommt dafür nur
     `CAP_DAC_READ_SEARCH` (lesen), sonst eng abgeschottet (systemd-Härtung), kein volles root.
3. **Verteilung:** Linux als `.deb`, Windows mit **Inno Setup** (wie openpacketmon). Signatur der Windows-Fassung
   später zusätzlich mit dem Certum-Open-Source-Zertifikat (braucht ein öffentliches Projekt).
4. **Neu anmelden statt Token übernehmen.**
   - opensecmon zeigt einen **Anmeldecode**, der Adresse, Fingerabdruck des Zertifikats und Einmalcode in einer
     Zeichenkette bündelt; der Anwender kopiert nur diesen einen Code (Windows: Feld im Installer).
   - Für die Rechner, die heute ein Skript nutzen: auf der Seite des Rechners „Neuer Code“. Die Anmeldung damit
     behält den Eintrag in opensecmon (Assets, Triage, „ignoriert“; geprüft in `storage/agents.rs` `new_code`
     und `enroll`), das alte Token wird ungültig. Der Installer entfernt das alte Skript (geplante Aufgabe bzw.
     systemd-Timer) selbst.
5. **Updates nur als Hinweis mit Knopf.**
   - Die Agent-Pakete (`.deb`, `setup.exe`) werden **im opensecmon-Paket mitgeliefert**; ein neuer Agent kommt
     also mit dem nächsten opensecmon-Update. Kein Internetzugriff der Rechner, das Repo muss dafür nicht
     öffentlich sein.
   - opensecmon zeigt je Rechner die Agent-Version und „Update verfügbar“; ein Knopf „Aktualisieren“ (je Rechner,
     evtl. auch für alle) setzt einen Auftrag, den der Agent mit der nächsten Antwort erfährt. Nicht automatisch.
   - Der Agent lädt das Paket über die bestehende, geprüfte Verbindung, **prüft die Signatur** (Punkt 6) und
     installiert erst dann: Linux über einen kleinen, getrennten Update-Helfer mit root (`dpkg -i`), nicht im
     Hauptdienst; Windows startet der SYSTEM-Dienst `setup.exe /VERYSILENT`. Danach sofort eine Meldung mit der
     neuen Version; Fehler stehen auf der Seite des Rechners.
6. **Signatur der Agent-Pakete (nur Herausgeber, für Anwender unsichtbar).**
   - Ed25519-Schlüssel mit Passphrase, erzeugt vom Nutzer selbst mit `ssh-keygen` (auf dem Laptop vorhanden, kein
     neues Werkzeug), abgelegt außerhalb der Repos, mit Sicherungskopie,
     Passphrase getrennt. Claude bekommt Schlüssel und Passphrase nie zu sehen.
   - Signiert wird beim Paketbau mit `ssh-keygen -Y sign` über den ssh-agent (`ssh-add` einmal je Sitzung); ohne
     geladenen Schlüssel bricht der Bau mit klarem Hinweis ab.
   - Der öffentliche Schlüssel steht fest im Agenten (im Repo). Passt die Signatur nicht, installiert der Agent
     nichts und meldet das an opensecmon. Selbst ein übernommenes opensecmon kann den Rechnern so nichts
     unterschieben.
   - Verlust: Agenten laufen weiter, nur der Knopf geht nicht mehr; einmal von Hand ein Paket mit neuem Schlüssel
     installieren. Wechsel: eine mit dem alten Schlüssel signierte Version bringt den neuen mit.
   - Selbst gebaute Agenten ohne diesen Schlüssel: Update-Knopf ausgeblendet.
   - Hardware-Schlüssel (YubiKey) nicht nötig, solange Updates nur auf Knopfdruck und nur über opensecmon kommen.

7. **Anmelden unter Linux** über einen kleinen Anmeldedialog im Startmenü („opensecagent anmelden“, Rechte über
   `pkexec`), keine Kommandozeile (Nutzer, 2026-11-07).
8. **Windows-Installer wird auf dem Windows-Laptop gebaut**, weil dort auch die Authenticode-Signatur (Certum)
   gemacht wird (Nutzer, 2026-11-07). Reihenfolge: auf dem Windows-Laptop bauen und mit Authenticode signieren,
   danach die eigene Ed25519-Signatur (Punkt 6) – die muss als Letztes kommen, weil Authenticode die Datei
   verändert. Vorschlag: dafür die fertige `setup.exe` auf den Linux-Laptop holen und dort signieren, damit der
   Schlüssel nur an einer Stelle liegt. Da die Pakete im opensecmon-Paket stecken, muss der Installer vor jedem
   opensecmon-Paket gebaut und signiert sein.
9. **Weitere Linux-Systeme** (Fedora, Arch …) nur angedacht, nicht geplant (Nutzer, 2026-11-07); opensecmon nimmt
   zurzeit nur Debian/Ubuntu an.

10. **Anmeldecode: Form A** (Nutzer, 2026-11-07) – eine Zeichenkette nur zum Kopieren.
    - Inhalt: Adresse von opensecmon (wie im Browser benutzt, auf der Seite änderbar), SHA-256-Fingerabdruck des
      Zertifikats (fehlt bei einem öffentlichen Zertifikat über einen Proxy, dann normale Prüfung), der heutige
      Einmalcode (8 Zeichen, 15 min, einmal), Formatversion und Prüfsumme gegen Kopierfehler.
    - Aussehen: `osa1_` plus rund 90 Zeichen (Unterstrich statt Bindestrich, siehe unten); das genaue Format wird in `docs/agent-api.md` von opensecmon
      festgelegt. Die Schnittstelle selbst bleibt, nur der Code wird zusammengepackt.
    - Bedienung: Knopf „Code kopieren“ in opensecmon; Installer und Linux-Dialog übernehmen einen Code aus der
      Zwischenablage selbst, wenn er mit `osa1_` beginnt. Fehlermeldungen in Anwendersprache („Code abgelaufen –
      in opensecmon einen neuen Code erzeugen“).
    - Form B (Adresse plus 16 Zeichen zum Abtippen, Server beweist sich mit dem Code) nicht jetzt; nur falls
      Abtippen doch gebraucht wird, später als Ergänzung.

## Nötige Änderungen in opensecmon (Schritt 7 dort, erledigt 2026-11-07)

Umgesetzt in opensecmon (noch nicht im Test-Paket): Formate in `docs/agent-api.md` (maßgeblich für den Agenten),
Anmeldecode mit Präfix `osa1_` (Unterstrich statt Bindestrich, damit ein Doppelklick alles markiert; Lesen als
Vorlage in `src/agent_code.rs`), Takt 15 min für `agent.name` = `opensecagent`, Update-Angebot in den Antworten,
Download `GET /api/v1/agent/packages/{file}`, `dev/agent-packages.sh` erwartet den öffentlichen Schlüssel als
`keys/allowed_signers` in diesem Repo (Identität und Namensraum `opensecagent`). Was der Agent dafür schicken muss:
`agent.name` `opensecagent`, `agent.updates: true` (nur mit eingebautem Herausgeber-Schlüssel), `os.arch`, bei
Fehlern `agent.update_error`.

Ursprüngliche Liste:

Erst in `docs/agent-api.md` festlegen, dann umsetzen (Eintrag in `../SecurityDashboad/PLAN.md`):
- Anmeldecode Form A (`osa1_…`, Adresse, Fingerabdruck, Einmalcode, Prüfsumme; Punkt 10), Knopf „Code
  kopieren“; Seite „Rechner hinzufügen“ mit
  Installer-Download und Code.
- `next_report_s` für den Agenten 15 min (die Skripte behalten 6 h).
- Agent-Version je Rechner, „Update verfügbar“, Knopf „Aktualisieren“, Auftrag in der Antwort auf die Meldung,
  Download der Pakete mit Signatur, Rückmeldung von Erfolg oder Fehler.
- Agent-Pakete und Signaturen im opensecmon-Paket (`dev/deb.sh`).

## Phasen (Vorschlag, je nach „go“)

0. **Repo und Plan** – erledigt (2026-11-07): Repo, Lizenz, README, CHANGELOG, CI (überspringt die Rust-Schritte bis
   zum ersten `Cargo.toml`), dieser Plan.
1. **Grundgerüst** – erledigt (2026-11-08, „go“ des Nutzers): Cargo-Projekt, `deny.toml`, Lizenzliste,
   Kommandozeile (`enroll`, `report`, `status`, `inventory`, `uninstall`, `version`), Dateien mit engen Rechten,
   HTTPS mit Fingerabdruck, Tests gegen eine Attrappe der API. Einzelheiten unten unter „Phase 1“.
2. **Linux** – erledigt (2026-11-08, „go“ des Nutzers): Inventar wie das Skript (dpkg, Snap, Flatpak auch der
   Benutzer, Kernel, Neustart, Pro), systemd-Dienst, `.deb`-Angaben, Anmeldedialog im Startmenü, Test gegen eine
   Wegwerf-Instanz von opensecmon. Einzelheiten unten unter „Phase 2“. Das Test-Paket wird nur auf Auftrag gebaut.
3. **Windows:** Inventar wie das Skript plus die Lücken oben (Benutzer ohne Anmeldung, Python install manager),
   Dienst, Inno-Setup-Installer mit Feld für den Anmeldecode (gebaut und signiert auf dem Windows-Laptop); Test auf
   dem Windows-Testrechner.
4. **Updates:** Signatur, Update-Helfer, Knopf in opensecmon (zusammen mit dem Schritt dort).
5. **Feinschliff:** Sicherheitsdurchsicht, Fuzztests, Doku, Übergang von den Skripten.

## Phase 1: Grundgerüst (erledigt 2026-11-08)

Umgesetzt:
- **Aufbau:** `src/code.rs` (Anmeldecode `osa1_…` lesen, wie `agent_code.rs` in opensecmon, Beispiel aus
  `docs/agent-api.md` als Test), `src/store.rs` (Dateien des Agenten), `src/http.rs` (HTTPS), `src/api.rs`
  (Formate und Fehler der API), `src/agent.rs` (Anmelden, Melden, Status; Tests mit Attrappe in
  `src/agent/tests.rs`), `src/inventory/` (Inventar), `src/main.rs` (Kommandozeile).
- **Dateien:** ein Ordner, Linux `/var/lib/opensecagent` (Ordner `700`, Dateien `600`), Windows
  `%ProgramData%\opensecagent`; `--dir` bzw. `OPENSECAGENT_DIR` für Tests. Darin `enrollment.json` (Adresse,
  Fingerabdruck, Name in opensecmon), `token` (nur `osm1_…` wird angenommen, `Debug` zeigt ihn nie),
  `state.json` (Hash und Zeit der letzten angenommenen Meldung, Abstand, „gesperrt“, letzter Fehler). Schreiben
  über eine temporäre Datei mit `rename`. Keine Konfigurationsdatei: alles kommt aus dem Anmeldecode.
- **HTTPS:** hyper 1 über rustls (ring) wie in opensecmon, HTTP/1.1, eine Verbindung je Anfrage, `Host` genau wie
  im Code (passt zu `allowed_hosts`), Zeitgrenzen (10 s Verbindung, 120 s Anfrage), Antworten höchstens 1 MB,
  `Authorization` als sensibel markiert. Zertifikat: Fingerabdruck aus dem Code **oder** gültig für die Adresse über
  den Zertifikatsspeicher des Systems (`rustls-platform-verifier`, damit auch eine Firmen-CA wie im Browser
  funktioniert; ohne aws-lc). Abgelehnt wird mit dem angebotenen Fingerabdruck, und es geht nichts raus (Test).
- **Melden:** JSON-Form der API, gzip; unverändert nur der Hash (Kurzform), bei `409` sofort die volle Liste;
  `401 invalid_token` setzt „gesperrt“ und hört auf, bis neu angemeldet wird; `429`/`503` mit `Retry-After`;
  Fehler in Anwendersprache („Code abgelaufen – in opensecmon einen neuen Code erzeugen“). `enroll` schickt
  gleich eine erste Meldung. `inventory` zeigt, was gesendet würde, ohne zu senden.
- **Inventar Linux (vorgezogen aus Phase 2, weil klein):** `/etc/os-release`, laufender Kernel, Neustart nötig,
  Ubuntu Pro, Paketliste von `dpkg-query` wie das Skript. Snaps und Flatpaks kamen in Phase 2 dazu. Windows
  meldet „noch nicht unterstützt“.
- **CI:** zusätzlich ein Windows-Job (clippy und Tests auf `windows-2025`; zählt im privaten Repo doppelt bei den
  Actions-Minuten) und die Prüfung von `THIRD-PARTY-LICENSES.txt` (`dev/third-party-licenses.py`, Vereinigung
  der Crates für Linux und Windows, x86_64 und arm64).
- **Abhängigkeiten:** clap, flate2 (zlib-rs), hyper, hyper-util, http-body-util, tokio, tokio-rustls (ring),
  rustls-platform-verifier, serde, serde_json, sha2, thiserror; zum Testen rcgen und tempfile. `cargo deny` grün.

Commits `6e45384`–`c72a8f7`, CI grün (Linux und Windows). Geprüft: 33 Tests (Code-Leser mit Beschädigungen, Dateirechte, Attrappe über HTTPS mit selbstsigniertem und
CA-Zertifikat: Anmeldung, volle Meldung, Kurzform, `409`, Sperre, Warten, Fehlerdokumente). Dazu einmal gegen
eine Wegwerf-Instanz von opensecmon (Stand `27394cc`, `127.0.0.1`, leere DB, ohne Quellen) mit echtem Anmeldecode
von der Seite des Rechners: Anmeldung, volle Meldung (149 KB, 1208 Quellpakete als Assets wie beim Skript),
Kurzform, `status`, verbrauchter Code abgelehnt, zweite volle Meldung innerhalb von 5 min mit `429` beantwortet.

Offen für spätere Phasen:
- Windows: Zugriffsliste des Ordners (nur SYSTEM und Administratoren) setzt der Installer (Phase 3); bis dahin
  erbt der Ordner die Rechte von `ProgramData`.
- ~~Dauerbetrieb mit dem Dienst in Phase 2~~ erledigt (Phase 2).
- `agent.updates`, Update-Angebot und `os.arch`-abhängige Pakete in Phase 4 (die Antwortfelder `update` werden
  gelesen, aber noch nicht genutzt).

## Phase 2: Linux (erledigt 2026-11-08)

Umgesetzt:
- **Snaps** (`src/inventory/snap.rs`): `snap list` mit `LC_ALL=C` (sonst übersetzt snap die Spalten), Herausgeber
  ohne `*`/`**`, `-` = keiner. Fehlt snap, gibt es keine Snaps; läuft snap, scheitert aber, wartet die Meldung (eine
  leere Liste würde die Snap-Assets in opensecmon entfernen).
- **Flatpaks** (`src/inventory/flatpak.rs`): **direkt aus den Installationsordnern** statt `flatpak list`, damit
  auch die Flatpaks aller Benutzer kommen, ohne `flatpak` als root in Ordnern der Benutzer laufen zu lassen.
  System (`/var/lib/flatpak`), weitere Installationen aus `/etc/flatpak/installations.d/*.conf`, je Benutzer
  (`/etc/passwd`, UID 1000–60000, dazu `/home/*`) `~/.local/share/flatpak`. Je App der aktive Stand
  (`app/<id>/<arch>/<zweig>/active`), Pflicht ist `metadata` mit `[Application]` und `name=<id>`; Name und
  Version aus `files/share/metainfo/<id>.metainfo.xml` (bzw. `appdata`), neueste `<release>` nach Zeitstempel.
  `scope` „machine“ bzw. „user“. Schutz: Dateien von Benutzer-Installationen müssen dem Benutzer gehören (sonst
  könnte ein Link z. B. `/etc/shadow` in die Meldung bringen), Öffnen mit `O_NONBLOCK` und Prüfung auf normale
  Datei (keine FIFO hält den Dienst an), Größengrenzen (64 KB `metadata`, 2 MB Metainfo), höchstens 1000
  Benutzerordner, 2000 Apps, 16 Zweige. Ergebnis auf diesem Laptop gleich `flatpak list` (Drawing 1.0.2,
  Mission Center 1.2.0).
- **Dienst** `opensecagent run` (`src/service.rs`): liest in jeder Runde die Dateien neu (eine Anmeldung aus dem
  Dialog greift nach spätestens einer Minute), meldet im Abstand, den opensecmon nennt (900 s, begrenzt auf
  60 s–24 h), plus bis zu 10 % Zufall; nach Fehlern 5, 15, dann 60 min; `429`/`503` wartet `Retry-After`; nicht
  angemeldet oder gesperrt: jede Minute nur die Dateien ansehen (kein Netz); nach dem Start 60 s Ruhe, wenn eine
  Meldung fällig ist; Ende mit SIGTERM; Ausgaben ins Journal. tokio dafür mit `macros` und `signal`.
- **Altes Skript entfernen** (`src/legacy.rs`): nach einer Anmeldung im Standardordner (also als root, nicht mit
  `--dir`) werden Timer und Dienst `opensecmon-agent`, `/etc/opensecmon-agent` und
  `/usr/local/sbin/opensecmon-agent` entfernt (wie `opensecmon-agent uninstall`), damit der Rechner nicht doppelt
  meldet. Ein Link anstelle des Ordners wird selbst entfernt, nie sein Ziel. In opensecmon steht der Rechner danach
  eventuell zweimal (alter Eintrag des Skripts); der Dialog sagt das.
- **Anmeldedialog** `opensecagent enroll-dialog` (`src/dialog.rs`, Startmenü „Bei opensecmon anmelden“): läuft als
  Benutzer; liegt ein Code in der Zwischenablage (`wl-paste`, `xclip` oder `xsel`, je nach Sitzung; 3 s Zeitgrenze),
  fragt er „… für opensecmon unter <Adresse>. Diesen Rechner damit anmelden?“, sonst Eingabefeld zum Einfügen.
  Prüft den Code vorab, dann `pkexec /usr/bin/opensecagent enroll --for-dialog` mit dem Code auf stdin (nie in der
  Prozessliste); die Antwort ist eine JSON-Zeile mit Fehlerart, der Dialog sagt sie auf Deutsch oder Englisch (aus
  `LC_ALL`/`LC_MESSAGES`/`LANG`) mit einem Rat, was zu tun ist. zenity (GNOME u. a.) oder kdialog (KDE); fremde
  Texte ohne `<`/`>` und für zenity als Pango maskiert. Abweichung: der Code wird nicht ins Eingabefeld
  vorbelegt (das ginge nur über die Kommandozeile von zenity), sondern per Rückfrage übernommen.
- **Paket** (`dev/deb.sh`, `packaging/linux/`, Angaben in `Cargo.toml`): statisches musl-Binary (3,4 MB),
  Dienst `opensecagent.service` (root, aber nur `CAP_DAC_READ_SEARCH`, `ProtectSystem=strict`,
  `ProtectHome=read-only`, `StateDirectory` mit `700`, `UMask=0077`, weitere Härtungen wie beim Skript), gleich
  aktiviert und gestartet (wartet bis zur Anmeldung), polkit-Aktion `io.github.tomcslk.opensecagent.enroll` mit
  `auth_admin` (Passwort jedes Mal), Startmenü-Eintrag und Symbol (Schild mit Pfeil nach oben im Stil von
  opensecmon). Abhängigkeiten `zenity | kdialog`, `pkexec | policykit-1`; empfohlen `wl-clipboard`,
  `xclip | xsel` (damit der Dialog den Code selbst findet). `purge` löscht `/var/lib/opensecagent`.

Geprüft: 49 Tests (u. a. Snap-Zeilen, Flatpak-Ordner mit Besitzerprüfung, Link auf `/etc/passwd`, FIFO,
Installationen, Wartezeiten, Fälligkeit mit zurückgestellter Uhr, Entfernen des Skripts in einem Wurzelordner,
Antworten für den Dialog, Sprache, Maskierung). Gegen eine Wegwerf-Instanz von opensecmon (`127.0.0.1`, leere DB,
ohne Quellen, danach gelöscht): `enroll --for-dialog` mit echtem Code (JSON-Zeile, erste Meldung, 1208 Assets, beide
Flatpaks in `agent_items`), verbrauchter, beschädigter und unerreichbarer Code mit der richtigen Fehlerart,
`run` mit fälliger Meldung (Kurzform nach 60 s, Ende mit SIGTERM). Statisches Binary gebaut und `inventory` damit
geprüft. Commits `9bcbdef`, `750f9da`, `36e15a3`, `d4af54e` (Doku) und `c5d33b3` (Windows-CI), CI grün (Linux und Windows).

**Test-Paket 0.1.0-1** (2026-11-08, auf Auftrag, `dev/deb.sh`, 1,4 MB, SHA-256 `bc076a19…`, kein Release) im
Container geprüft: `debian:trixie` mit systemd als Init (Image `opensecagent-debtest`), Netz des Rechners, damit eine
Wegwerf-Instanz von opensecmon auf `127.0.0.1` erreichbar war (danach gelöscht). Ergebnis:
- `apt install ./…deb` zieht `zenity` und `pkexec` nach; Dienst aktiviert und gestartet, `/var/lib/opensecagent`
  mit `700`; `systemd-analyze security` 3,5 („OK“).
- `opensecagent enroll` als root mit dem Code auf stdin: angemeldet, erste Meldung, ein nachgebautes altes Skript
  (Timer aktiv, `/etc/opensecmon-agent`, `/usr/local/sbin/opensecmon-agent`) vollständig entfernt, Timer danach
  inaktiv; Dateien `600`; `status` als Benutzer verweigert mit Hinweis auf `sudo`.
- Ein Flatpak im Home-Ordner eines Benutzers (Ordner `700`) kam als `scope = user` an. Der gehärtete Dienst sah
  danach dieselbe Liste wie root (nur der Hash ging raus).
- `apt remove` stoppt den Dienst und lässt die Anmeldung liegen, `apt purge` löscht `/var/lib/opensecagent`;
  `--reinstall` startet den Dienst neu.

Befund dabei: Im rootless Container fehlt `CAP_DAC_READ_SEARCH` in der Begrenzungsmenge des Containers, dann hat
der Dienst gar keine Fähigkeit, liest die Benutzer-Flatpaks nicht und meldet sie **still** nicht mehr (opensecmon
würde ihre Assets entfernen). Auf einem normalen Rechner tritt das nicht auf; mit `--cap-add DAC_READ_SEARCH`
verhielt sich der Container wie ein echter Rechner.

**Umgesetzt (2026-11-08, „go“ des Nutzers):** Gibt es den Flatpak-Ordner eines Benutzers, kann ihn der Agent aber
nicht lesen (`has_apps` in `src/inventory/flatpak.rs`: der Ordner selbst oder sein `app`-Ordner liefert einen
anderen Fehler als „fehlt“), wird die Meldung zurückgehalten; der Dienst schreibt den Grund ins Journal („the
inventory could not be read, the report waits: … Permission denied … the service needs the capability
CAP_DAC_READ_SEARCH“) und versucht es nach 5, 15, 60 min wieder. Ausnahme: Home-Ordner auf einem Netzlaufwerk (Typ
aus `/proc/self/mountinfo`: NFS, CIFS/SMB, Ceph, GlusterFS, AFS, Lustre, GPFS, BeeGFS, 9p, sshfs, davfs, OCFS2,
GFS2), weil root sie bei NFS mit `root_squash` nie lesen darf und der Rechner sonst gar nicht mehr meldete: diese
werden einmal je Dienstlauf im Journal genannt und ausgelassen. Folge für die Kommandozeile: `inventory` und
`report` brauchen auf Rechnern mit mehreren Benutzern root. Im Container geprüft (neues Binary über dem Test-Paket
0.1.0-1): ohne die Fähigkeit Fehler mit Hinweis, mit ihr das Flatpak des Benutzers; 51 Tests; Commits `f7b0e93`,
`33b6af5` (Test mit echtem Rechner ohne root), CI grün. Enthalten in **Test-Paket 0.1.0-2** (2026-11-08, auf Auftrag, aus `bd124b2`, `dev/deb.sh --deb-revision 2`, SHA-256
`4c8346bc…`, kein Release), im Container geprüft mit zwei Rechnern gegen eine Wegwerf-Instanz (danach gelöscht):
- Mit `CAP_DAC_READ_SEARCH`: 0.1.0-1 installiert, angemeldet (altes Skript entfernt), auf 0.1.0-2 aktualisiert:
  Dienst neu gestartet, Anmeldung behalten, Score 3,5. Die nächste Meldung war voll (die Paketliste enthält
  `opensecagent` selbst, nach dem Update also geändert), opensecmon ließ sie erst nach 5 min zu (`429`, der Dienst
  wartete die genannten 229 s); das Flatpak des Benutzers blieb erhalten. `remove`/`purge` wie bei 0.1.0-1.
- Ohne die Fähigkeit (`CapBnd` 0): Anmeldung als root klappte, danach schrieb der Dienst den Grund ins Journal und
  meldete nicht; das Flatpak blieb in opensecmon erhalten.

Befund: Ein Inventarfehler im Dienst landet nur im Journal, nicht in `last_error` von `state.json`; `opensecagent
status` zeigt dann „Connection: working“ ohne Hinweis. **Umgesetzt (2026-11-08, „go“ des Nutzers):**
`Agent::note_error` speichert einen Fehler vor dem Senden (Inventar nicht lesbar) als `last_error`, beim Dienst,
bei `report` und bei der ersten Meldung nach der Anmeldung; `status` zeigt ihn als „Last error“, die nächste
angenommene Meldung löscht ihn; ohne Anmeldung wird nichts geschrieben. 52 Tests (neu: Fehler bleibt bis zur
nächsten Meldung, übriger Zustand unverändert). Commit `dcead72`, CI grün.

**Test-Paket 0.1.0-3** (2026-11-08, auf Auftrag, aus `dcead72`, `dev/deb.sh --deb-revision 3`, SHA-256 `3cad4707…`, kein
Release), im Container mit zwei Rechnern gegen eine Wegwerf-Instanz geprüft (danach gelöscht):
- Ohne `CAP_DAC_READ_SEARCH`: 0.1.0-3 installiert und angemeldet (altes Skript entfernt); der Dienst meldete nicht,
  `status` zeigte den Grund als „Last error“ bei funktionierender Verbindung; eine Meldung von root
  (`opensecagent report`, root hat im Container die vollen Leserechte) ging als Kurzform durch und löschte ihn.
- Mit der Fähigkeit: 0.1.0-2 installiert und angemeldet, auf 0.1.0-3 aktualisiert (Dienst neu gestartet, Anmeldung
  behalten, Score 3,5); der Dienst wartete nach dem Update die `429` von opensecmon ab und schickte dann die volle
  Liste, kein „Last error“; das Flatpak des Benutzers blieb erhalten. `remove`/`purge` wie zuvor.

**Version 0.1.1** (2026-11-08, auf Auftrag, für die Installation auf dem Laptop; Commit `7f18e0d`, gleicher Code wie
0.1.0-3): Paket `opensecagent_0.1.1-1_amd64.deb`, SHA-256 `b7faf25a…`, kein Release. In einem Container mit
Ubuntu 24.04 und systemd installiert (Grundlage von Zorin OS 18): `pkexec`, `zenity`, `wl-clipboard` und `xclip` kamen
mit, Dienst läuft und wartet auf die Anmeldung, Score 3,5. Voraussetzung für die Anmeldung: opensecmon mit Schritt 7
(Anmeldecode `osa1_…`, ab Commit `e259827`); das Test-Paket 0.3.3 auf dem Server hat ihn noch nicht. Für einen
Rechner, der schon mit dem Skript meldet, den Code auf seiner Seite mit „Neuen Code erzeugen“ holen: dann bleibt es
ein Eintrag mit seinen Assets.

Auf dem Laptop (Zorin OS 18, GNOME/Wayland) vom Nutzer installiert und über den Dialog „Bei opensecmon anmelden“
angemeldet (2026-11-08, nach opensecmon 0.3.4 auf dem Server): Dienst seit 14:12 aktiv, altes Skript (Timer, Programm,
`/etc/opensecmon-agent`) entfernt. Damit sind das echte polkit-Passwortfenster und die Installation auf dem Laptop
geprüft. Befund: Der Code musste von Hand eingefügt werden, weil er erst nach dem Öffnen des Fensters kopiert wurde
(das Fenster schaut nur beim Start in die Zwischenablage). Nachgestellt mit dem schon kopierten Code: Die Frage
„Code aus der Zwischenablage verwenden?“ kommt, `wl-paste` funktioniert also unter GNOME/Wayland auch beim Start aus
dem Startmenü. Umgesetzt nach „go“ (2026-11-08): OK mit leerem Feld schaut noch einmal in die Zwischenablage (mit derselben Frage;
ohne Code dort ein Hinweis, wie man ihn kopiert), der Hinweistext nennt das und für einen schon bekannten Rechner
„Neuen Code erzeugen“; `offered_code` in `src/dialog.rs`, 53 Tests, Ablauf mit nachgebauten `zenity`/`wl-paste`
durchgespielt. **Version 0.1.2** auf Auftrag gebaut (2026-11-08, `opensecagent_0.1.2-1_amd64.deb`, SHA-256
`e2f13ba4…`, in `dist/0.1.2/`), im Container (Debian 13 mit systemd) als Update von 0.1.1 geprüft: Anmeldedateien
bleiben, Dienst startet als 0.1.2 neu, neuer Hinweistext im Programm. Vom Nutzer signiert (2026-11-08, Signatur geprüft) und mit `dev/agent-packages.sh` in opensecmon 0.3.5
übernommen (ohne 0.1.1); dort im Container geprüft: Download 0.1.2 auf der Seite des Rechners mit passender Prüfsumme.
Herausgeber-Schlüssel vom Nutzer angelegt (2026-11-08, Ed25519 mit Passphrase, öffentlicher Teil in
`keys/allowed_signers`, Commit `d3a478f`, Fingerabdruck `SHA256:vBp3hrefqGf4vOiqlQKoHyR6BwFdf+v2E0PC1WjBRXs`), 0.1.1 vom
Nutzer signiert (`dist/0.1.1/`, gitignored) und mit `dev/agent-packages.sh` in opensecmon 0.3.4 übernommen; dort im
Container geprüft: Download von der Seite des Rechners, Anmeldung als derselbe Rechner, Takt 15 min, altes Token 401.

## Phase 3: Windows (Plan, Umsetzung nach „go“)

Vorbereitet am 2026-11-08 für die Weiterarbeit auf dem Windows-Rechner; Einrichten, Test-Instanz und Signatur-Ablauf
in `docs/windows.md`. Vorlage für das Inventar ist `contrib/opensecmon-agent.ps1` in opensecmon (Funktionen
`Get-OsInfo`, `Get-Programs`, `Get-StoreApps`); das Ziel-Format steht in `docs/agent-api.md` dort („Windows“, Felder
`os.*` und `items[]` mit `type` `win`/`appx`). opensecmon nimmt Windows-Meldungen schon an (seit dem Skript) und
erwartet das Paket als `opensecagent-<version>-<x64|arm64>-setup.exe`; dort ist für Phase 3 nichts zu ändern.

Vorschlag in Schritten, je mit Tests, Commit und Bericht:

- **3a Inventar Windows** (`src/inventory/windows.rs`):
  - `os`: `ProductName`, `DisplayVersion`, `CurrentBuild`, `UBR`, `InstallationType` aus
    `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`; `os.arch` (`x64`/`arm64`).
  - `win`-Einträge aus `…\CurrentVersion\Uninstall` (64 und 32 Bit) und aus den Hives angemeldeter Benutzer unter
    `HKEY_USERS` (ohne `_Classes`), mit denselben Filtern wie das Skript (`SystemComponent = 1`, `ParentKeyName`,
    `ReleaseType` Update/Hotfix/Security Update), `scope` machine/user, ohne Benutzernamen.
  - `appx`: Store-Apps aller Benutzer ohne Frameworks und Ressourcenpakete (das Skript nutzt
    `Get-AppxPackage -AllUsers`).
  - Lücken des Skripts schließen: Pythons aus dem „Python install manager“ über die PEP-514-Einträge
    (`Software\Python\<Firma>\<Tag>`, Maschine und Benutzer).
  - Grenzen wie unter Linux (`field`, Höchstzahlen); Registry-Werte sind Daten.
  - Tests mit nachgebauten Werten (die Lesefunktionen bekommen eine Quelle, die sich im Test ersetzen lässt).
- **3b Benutzer ohne Anmeldung:** `NTUSER.DAT` nicht angemeldeter Benutzer (Profile aus
  `HKLM\…\ProfileList`) **nur als Datei lesen, nicht einhängen** (Entschieden, Punkt 2). Datei mit
  Freigabe-Modus öffnen, der Windows nicht stört; Hive-Format selbst lesen (nur Schlüssel und Zeichenkettenwerte unter
  `…\Uninstall`) oder eine Crate mit passender Lizenz; ein nicht lesbarer Hive wird ausgelassen und im Ereignisprotokoll
  genannt (anders als bei Linux-Flatpaks gibt es hier keinen sicheren Weg, die Meldung zurückzuhalten, ohne dass ein
  gesperrter Hive den Rechner dauerhaft stumm macht; Entscheidung des Nutzers). Fuzztest für den Hive-Leser.
- **3c Dienst:** `opensecagent run` als Windows-Dienst (LocalSystem, Start automatisch verzögert); Stop über den
  Dienst-Manager statt SIGTERM (`stop_signal` in `src/service.rs`); Ausgaben ins Ereignisprotokoll oder eine
  Log-Datei im Ordner des Agenten. Ordner `%ProgramData%\opensecagent` mit eigener Zugriffsliste nur für SYSTEM und
  Administratoren, ohne Vererbung (setzt der Agent selbst beim Anlegen, wie `Protect-Dir` im Skript; offen aus
  Phase 1).
- **3d Altes Skript entfernen** (`src/legacy.rs` für Windows): geplante Aufgabe `opensecmon-agent` und Ordner
  `%ProgramData%\opensecmon-agent` (wie `Invoke-Uninstall` im Skript), nur nach einer Anmeldung im Standardordner.
- **3e Installer** (Inno Setup, `packaging/windows/`, `dev/build-installer.ps1`, nur auf Auftrag bauen): Installation
  für alle Benutzer nach `Program Files`, Dienst einrichten und starten, Seite mit Feld für den Anmeldecode (vorbelegt,
  wenn die Zwischenablage mit `osa1_` beginnt; leer lassen = später anmelden), Anmeldung ohne den Code in der
  Kommandozeile (z. B. über eine Datei nur für SYSTEM/Administratoren oder die Eingabe des Programms), Meldungen in
  Anwendersprache (Deutsch/Englisch); Deinstallation entfernt Dienst und Anmeldung. Programm mit Icon und
  Versionsangaben (`winresource` als Build-Abhängigkeit, wie openpacketmon). `/VERYSILENT` muss für Phase 4 gehen.
  Startmenü-Eintrag für eine spätere Neuanmeldung.
  - **Erste Stufe erledigt (2026-11-08, „go“ des Nutzers: zuerst 3e als Probelauf der Signatur):**
    `packaging/windows/opensecagent.iss` (für alle Benutzer nach `Program Files`, nur Programm, Lizenzen und README;
    noch ohne Dienst, Code-Feld und Startmenü), `dev/build-installer.ps1` (Bau, Lizenzliste, Inno Setup; mit
    `-CertSha1` signiert es `opensecagent.exe`, Installer, Deinstaller und die temporären Kopien von Setup mit
    Zeitstempel von Certum und prüft die Signaturen), `build.rs` mit `winresource` (Icon, Versionsangaben),
    `dev/make-ico.ps1` (Icon aus dem SVG über Edge), C-Laufzeit statisch (`.cargo/config.toml`). Probe-Installer
    `opensecagent-0.1.2-x64-setup.exe` gebaut und mit dem Certum-Zertifikat signiert, `signtool verify /pa`
    erfolgreich; Installation mit Smart App Control macht der Nutzer.
- **3f Prüfen:** Test-Instanz in WSL (`docs/windows.md` Abschnitt 5), saubere Windows-Umgebung für den Installer,
  dann Installation auf dem eigenen Windows-Rechner durch den Nutzer (Wechsel vom Skript auf opensecagent mit
  „Neuen Code erzeugen“, damit der Rechner derselbe Eintrag bleibt). Danach README, CHANGELOG, Version.

Offen (vor oder bei den Schritten zu klären):
1. Store-Apps: über die Windows-API (`PackageManager`, Crate `windows`) oder über `Get-AppxPackage` als Hilfsprozess?
   Vorschlag: API, damit kein PowerShell-Prozess mit Ausgabe-Zerlegung nötig ist.
2. Hive-Leser selbst schreiben oder Crate (Lizenz gegen `deny.toml` prüfen)?
3. Neuanmeldung nach der Installation: Installer erneut starten (Reparatur mit Code-Feld), ein kleiner eigener Dialog
   oder nur `opensecagent enroll` als Administrator? Vorschlag: Startmenü-Eintrag, der einen kleinen Dialog öffnet
   (wie unter Linux: Code aus der Zwischenablage, Rückfrage, Rechte über UAC).
4. arm64-Fassung (`aarch64-pc-windows-msvc`) gleich mitbauen oder später?
5. Neue Abhängigkeiten (voraussichtlich `windows-service`, `windows`/`windows-registry`, `winresource`) jeweils mit
   Begründung und Lizenzprüfung.

## Vor dem Öffentlich-Schalten (2026-11-08)

Geprüft zusammen mit opensecmon: keine Geheimnisse in Stand und Historie (nur der öffentliche Schlüssel in
`keys/allowed_signers` und erfundene Beispielcodes). Betriebsdaten der eigenen Rechner und der Ort des
Signaturschlüssels nur in `CLAUDE.local.md`. README mit Stand, `SECURITY.md` und `CONTRIBUTING.md` neu. Lizenz
AGPL-3.0-or-later bestätigt. Die Historie wird wie bei opensecmon mit einem Anfangs-Commit neu begonnen (alte
Rechnernamen standen in früheren Commits); Commit-Nummern in diesem Plan beziehen sich auf die frühere, private
Historie.

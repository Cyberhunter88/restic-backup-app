# restic-backup-app

Desktop-App für [restic](https://restic.net)-Backups auf ein NAS (SMB-Freigabe) mit Zeitplan, Wiederherstellung und Aufbewahrungsregeln. Läuft unter Windows und Linux.

## Funktionen
- Backup auf SMB-Freigabe (Windows: UNC-Pfad, Linux: `mount.cifs`)
- Bestehende restic-Repos weiterverwenden oder neue anlegen
- Ausschlüsse: Muster, `CACHEDIR.TAG`, Markerdateien (`.nobackup`), Größenlimit
- Zeitplan mit mehreren Uhrzeiten pro Tag und Wochentagen (Windows: Aufgabenplanung, Linux: systemd-User-Timer), Verlauf der letzten Läufe
- Snapshots durchsuchen, Dateien oder ganze Snapshots wiederherstellen: in einen Zielordner oder am Originalort, mit Fortschritt, Abbrechen und Wahl, was mit vorhandenen Dateien geschieht (Originalort und diese Wahl brauchen restic ab 0.17)
- Warnung, wenn der freie Platz auf dem NAS unter eine einstellbare Schwelle fällt
- Aufbewahrungsregeln (täglich/wöchentlich/monatlich) mit automatischem `forget --prune`
- Autostart beim Anmelden (optional); verpasste Zeitplan-Läufe werden nachgeholt, auch nach mehreren Tagen Pause
- Laptop-tauglicher Zeitplan: sichert nur, wenn ein Backup fällig ist (kein Doppel-Backup nach dem Aufwachen), überspringt still, wenn das NAS unterwegs nicht erreichbar ist, optional nur am Netzteil; Pause-Funktion und Meldung „überfällig“ nach einstellbar vielen Tagen
- Update-Suche (GitHub-Releases) im Info-Tab
- Passwörter liegen im System-Schlüsselbund, nicht in der Konfigurationsdatei

## Installation
Die aktuellen Pakete stehen unter [Releases](../../releases/latest).

| System | Datei | Hinweis |
|---|---|---|
| Windows | `restic-backup-app-setup-<version>.exe` | Installer inkl. restic, Startmenü-Eintrag |
| Windows | `restic-backup-app-windows-x86_64.exe` | portabel, restic muss im PATH liegen |
| Debian/Ubuntu | `restic-backup-app_<version>_amd64.deb` | `sudo apt install ./restic-backup-app_*.deb` (zieht `restic`, `cifs-utils`, `policykit-1` nach) |
| Arch Linux | `packaging/arch/install.sh` | baut und installiert ein pacman-Paket, siehe unten |
| Andere Linux | `restic-backup-app-<version>-x86_64.AppImage` | `chmod +x` und starten; `restic` und `cifs-utils` müssen installiert sein |

### Arch Linux
```
git clone https://github.com/Cyberhunter88/restic-backup-app.git
cd restic-backup-app
packaging/arch/install.sh
```
Das Skript holt die Build-Abhängigkeiten, baut ein pacman-Paket (`restic`, `cifs-utils`, `polkit`, `gtk3` werden mitinstalliert) und installiert es nach `/usr/bin` mit Menüeintrag. Update: dasselbe Skript noch einmal ausführen (macht `git pull` und baut neu); entfernen mit `sudo pacman -R restic-backup-app`.

## Update
- **Arch:** `packaging/arch/install.sh` im geklonten Repo.
- In der App: Tab **Info → Nach Updates suchen** (beim Start einmal täglich automatisch) meldet neue Versionen; **Neue Versionen ansehen** öffnet die Releases-Seite. Nach einem Update den Zeitplan einmal neu aktivieren.
- **Windows:** neues Setup einfach über die alte Installation starten. Einstellungen und Passwörter bleiben erhalten.
- **Debian/Ubuntu:** neue `.deb` mit `sudo apt install ./restic-backup-app_*.deb` installieren.
- **AppImage:** neue Datei herunterladen und die alte ersetzen.

## Einrichtung
1. **Einstellungen:** NAS-Host, Freigabe, Unterordner des Repos, SMB-Benutzer/-Passwort, Repo-Passwort und Quellordner eintragen, speichern.
2. **Bestehendes Repo nutzen:** Unterordner auf das vorhandene Repo setzen, Repo-Passwort eintragen und **Repo prüfen** klicken. Ein neues Repo wird nur angelegt, wenn „Neues Repo anlegen“ aktiviert ist.
3. **Ausschlüsse:** Muster wie `*.tmp` oder `node_modules`; „Übliche Ausschlüsse“ fügt eine Vorlage hinzu.
4. **Zeitplan:** Uhrzeiten (z. B. „Alle 6 Stunden“) und Wochentage wählen, dann „Zeitplan aktivieren“. Unter Linux fragt die App dabei einmalig nach dem Benutzer-Passwort und trägt die Freigabe in `/etc/fstab` ein; danach wird ohne Rückfrage eingebunden, auch bei geplanten Läufen.

Das Ergebnis des letzten Laufs steht im Backup-Tab (`last-run.log` im Datenverzeichnis der App).

## Entwicklung
```
cargo test
cargo build --release
```
Voraussetzungen: Rust (stable), restic im PATH (für die Integrationstests). Linux zusätzlich: `libdbus-1-dev pkg-config libxkbcommon-dev libwayland-dev libgtk-3-dev`.

### Workflow: nie direkt auf `main`
Änderungen laufen immer über einen Branch und einen Pull Request:
1. Branch anlegen: `git switch -c feature/<thema>`
2. Committen, pushen, Pull Request gegen `main` öffnen.
3. Der Build-Workflow (Tests, Linux- und Windows-Pakete) muss grün sein, dann wird gemerged.

Einmalig nach dem Klonen aktivieren: `git config core.hooksPath .githooks` – der `pre-push`-Hook blockiert Pushes direkt auf `main`. (Zusätzlich ist `main` auf GitHub per Branch-Protection geschützt.)

### Release
Wie beim cctv-viewer: Die Version wird im Pull Request erhöht, das Release startest du danach von Hand. Der Release-Workflow ändert nichts an `main` und braucht keine PR-Berechtigung.
1. **Im Feature-PR:** `version` in `Cargo.toml` erhöhen und `cargo update --workspace` (zieht `Cargo.lock` nach). Der Changelog ist optional: Wer will, benennt `## [Unveröffentlicht]` in `## [X.Y.Z]` um, das wird dann der Release-Text. Nur Versionen im Format `X.Y.Z`.
2. PR mergen, danach baut `build.yml` den `main`-Commit (Windows-Setup, portable `.exe`, `.deb`, AppImage; die Pakete bleiben 90 Tage als Artefakte).
3. Wenn der Build grün ist: GitHub → **Actions** → **release** → **Run workflow** (Branch `main`), ohne Eingaben – die Version kommt aus der `Cargo.toml` von `main`, `Cargo.lock` muss dazu passen.
4. Der Workflow prüft, dass `Cargo.lock` zur Version passt und dass genau dieser `main`-Commit einen erfolgreichen Build hat. Dann legt er das Release als Entwurf an (Text aus dem Changelog, falls ein Abschnitt zur Version existiert), hängt die **unveränderten** gebauten Pakete plus `SHA256SUMS.txt` an und veröffentlicht erst danach. „Neueste“ rückt nur vor, wenn die Version die höchste ist.

Wiederholung mit derselben Version und demselben Commit ist unschädlich (nichts zu tun bzw. ein halb fertiger Entwurf wird fortgesetzt); ein Tag, der auf einen anderen Commit zeigt, wird abgelehnt. Hat ein `main`-Commit keinen Build (reine Doku-Änderung), vorher **Actions → build → Run workflow** auf `main` starten.

## Lizenz
MIT – siehe [LICENSE](LICENSE).

# restic-backup-app

Desktop-App für [restic](https://restic.net)-Backups auf ein NAS (SMB-Freigabe) mit Zeitplan, Wiederherstellung und Aufbewahrungsregeln. Läuft unter Windows und Linux.

## Funktionen
- Backup auf SMB-Freigabe (Windows: UNC-Pfad, Linux: `mount.cifs`)
- Bestehende restic-Repos weiterverwenden oder neue anlegen
- Ausschlüsse: Muster, `CACHEDIR.TAG`, Markerdateien (`.nobackup`), Größenlimit
- Zeitplan mit mehreren Uhrzeiten pro Tag und Wochentagen (Windows: Aufgabenplanung, Linux: systemd-User-Timer), Verlauf der letzten Läufe
- Snapshots durchsuchen, Dateien oder ganze Snapshots wiederherstellen
- Aufbewahrungsregeln (täglich/wöchentlich/monatlich) mit automatischem `forget --prune`
- Autostart beim Anmelden (optional); verpasste Zeitplan-Läufe werden nachgeholt, auch nach mehreren Tagen Pause
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
Das Skript holt die Build-Abhängigkeiten, baut ein pacman-Paket (`restic`, `cifs-utils`, `polkit`, `gtk3` werden mitinstalliert) und installiert es nach `/usr/bin` mit Menüeintrag. Update: dasselbe Skript noch einmal ausführen (macht `git pull` und baut neu); entfernen mit `sudo pacman -R restic-backup-app`. Das Repo ist privat, daher muss `git clone` bzw. `git pull` angemeldet sein (SSH-Key oder `gh auth login`).

## Update
- **Arch:** `packaging/arch/install.sh` im geklonten Repo.
- In der App: Tab **Info → Neue Versionen ansehen** öffnet die Releases-Seite (das Repo ist privat, deshalb ist dafür die GitHub-Anmeldung im Browser nötig).
- **Windows:** neues Setup einfach über die alte Installation starten. Einstellungen und Passwörter bleiben erhalten.
- **Debian/Ubuntu:** neue `.deb` mit `sudo apt install ./restic-backup-app_*.deb` installieren.
- **AppImage:** neue Datei herunterladen und die alte ersetzen.

## Einrichtung
1. **Einstellungen:** NAS-Host, Freigabe, Unterordner des Repos, SMB-Benutzer/-Passwort, Repo-Passwort und Quellordner eintragen, speichern.
2. **Bestehendes Repo nutzen:** Unterordner auf das vorhandene Repo setzen, Repo-Passwort eintragen und **Repo prüfen** klicken. Ein neues Repo wird nur angelegt, wenn „Neues Repo anlegen“ aktiviert ist.
3. **Ausschlüsse:** Muster wie `*.tmp` oder `node_modules`; „Übliche Ausschlüsse“ fügt eine Vorlage hinzu.
4. **Zeitplan:** Uhrzeiten (z. B. „Alle 6 Stunden“) und Wochentage wählen, dann „Zeitplan aktivieren“. Unter Linux die angezeigte `/etc/fstab`-Zeile eintragen, damit geplante Läufe ohne pkexec-Dialog mounten können.

Das Ergebnis des letzten Laufs steht im Backup-Tab (`last-run.log` im Datenverzeichnis der App).

## Entwicklung
```
cargo test
cargo build --release
```
Voraussetzungen: Rust (stable), restic im PATH (für die Integrationstests). Linux zusätzlich: `libdbus-1-dev pkg-config libxkbcommon-dev libwayland-dev libgtk-3-dev`.

### Workflow: nie direkt auf `master`
Änderungen laufen immer über einen Branch und einen Pull Request:
1. Branch anlegen: `git switch -c feature/<thema>`
2. Committen, pushen, Pull Request gegen `master` öffnen.
3. Der Build-Workflow (Tests, Linux- und Windows-Pakete) muss grün sein, dann wird gemerged.

Einmalig nach dem Klonen aktivieren: `git config core.hooksPath .githooks` – der `pre-push`-Hook blockiert Pushes direkt auf `master`. (GitHub-Branch-Protection ist bei privaten Repos im Free-Plan nicht verfügbar; sie lässt sich nachrüsten, sobald das Repo öffentlich ist oder ein Pro-Plan vorhanden ist.)

### Release
Release läuft vollautomatisch über GitHub Actions, immer ausgehend von `master`:
1. GitHub → **Actions** → Workflow **release** → **Run workflow** (Branch `master`), Versionsnummer eingeben (z. B. `0.3.1`).
2. Der Workflow prüft die Version (X.Y.Z, größer als die aktuelle, Tag noch frei), setzt sie in `Cargo.toml`/`Cargo.lock` und benennt den Abschnitt „Unveröffentlicht“ im `CHANGELOG.md` um.
3. Der Stand wird auf `release/vX.Y.Z` gebaut und getestet (Windows-Setup, portable `.exe`, `.deb`, AppImage).
4. Nur wenn alles grün ist: `master` wird auf diesen Stand vorgespult, Tag `vX.Y.Z` und GitHub-Release mit allen Paketen entstehen, der Release-Branch wird gelöscht.

Neue Änderungen vorher im `CHANGELOG.md` unter `## [Unveröffentlicht]` sammeln. Wird `master` später geschützt, ein Personal Access Token mit Schreibrecht (Bypass für den Schutz) als Secret `RELEASE_TOKEN` hinterlegen. Ein manuell gesetzter Tag `v*` löst weiterhin den Build samt Release aus.

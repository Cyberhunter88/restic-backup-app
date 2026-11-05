# Changelog

## [0.3.2]

### Neu
- Ausschlüsse: Das Eingabefeld nimmt jetzt mehrere Muster auf einmal an (ein Muster pro Zeile einfügen, Leerzeilen und `#`-Kommentare werden ignoriert, Doppelte übersprungen).
- „Übliche Ausschlüsse“ deckt deutlich mehr ab: Temp-/Cache-Ordner, AppData (Temp, INetCache, Packages, Browser-Caches), `NTUSER.DAT*`, Windows-Systemdateien sowie Linux-Papierkorb und -Caches.

### Behoben
- Windows: Zeitplan aktivieren schlug bei mehreren Uhrzeiten fehl („New-ScheduledTaskTrigger … System.Object[]“); die Trigger werden jetzt korrekt geklammert.
- Backup: Meldet restic nach geschriebenem Snapshot einen Fehlercode (z. B. „Zugriff verweigert“ bei gesperrten Ordnern unter Windows), gilt der Lauf als Warnung statt als Fehler.

## [0.3.1]

### Neu
- Autostart: Option „Programm beim Anmelden automatisch starten (minimiert)“ im Tab Zeitplan (Linux: `~/.config/autostart`, Windows: Run-Schlüssel).
- Verpasste Backups werden nachgeholt, auch wenn der PC mehrere Tage aus war: Linux über `Persistent=true` im Timer, Windows über „StartWhenAvailable“ (Task jetzt per PowerShell mit einem Trigger je Uhrzeit angelegt).
- Zeitplan-Läufe versuchen die NAS-Verbindung bis zu 5-mal (je 60 s Abstand), falls das Netzwerk direkt nach dem Start noch fehlt.
- Backup-Tab zeigt das Alter des letzten Laufs und warnt ab 2 Tagen („Überfällig“).
- Arch Linux: `packaging/arch/PKGBUILD` und `install.sh` zum Installieren und Aktualisieren als pacman-Paket.

### Behoben
- Zeitplan bei AppImage: Der Timer startet jetzt die AppImage-Datei (`$APPIMAGE`) statt eines temporären Mount-Pfads, der nach dem Beenden verschwindet.
- GitHub-Workflows: Pull Requests wurden doppelt gebaut (Push + PR), Release-Commits und reine Doku-Änderungen lösen keinen unnötigen Build mehr; veraltete Läufe werden abgebrochen.

## [0.3.0]

### Neu
- Zeitplan: mehrere Uhrzeiten pro Tag (Schnellauswahl 1×/2× täglich, alle 6/3 Stunden, stündlich) und Wochentage wählbar. Linux: mehrere `OnCalendar`-Einträge im systemd-Timer; Windows: ein Task je Uhrzeit. Ältere Einzel-Uhrzeit wird übernommen.
- Lokaler Verlauf der letzten Läufe (`history.tsv` im Datenverzeichnis, max. 200 Einträge), im Backup-Tab sichtbar – auch für Läufe des Zeitplans.
- Neue Oberfläche: dunkles Theme mit Akzentfarbe, Seitenleiste, Karten-Layout, farbige Statusmeldungen.
- Backup-Tab: Status-Badge des letzten Laufs, Kennzahlen, Fortschrittskarte, farbiges Protokoll.
- Snapshots: automatisches Laden, Filterfeld, Tabelle mit lesbarer Zeit, Dateigrößen in KB/MB/GB, Link zum Zielordner nach Wiederherstellung.
- Einstellungen in Bereiche gegliedert, Speichern-Leiste bleibt unten sichtbar, Enter fügt Ausschlüsse hinzu.

### Behoben
- Das Protokoll des letzten Laufs wird nicht mehr bei jedem Frame von der Platte gelesen.

## [0.2.1]

### Behoben
- Windows-Setup lieferte nur einen Chocolatey-Shim statt der echten `restic.exe` mit; die App findet jetzt das mitgelieferte restic und fällt auf das restic aus dem PATH zurück, falls es nicht startet.

## [0.2.0]

### Neu
- Installer: Windows-Setup (Inno Setup, bringt restic mit), Linux `.deb` und AppImage; Release-Workflow bei Tag `v*`.
- Bestehende restic-Repos verwenden: Button „Repo prüfen“ (vorhanden / nicht vorhanden / Passwort falsch); ein neues Repo wird nur angelegt, wenn „Neues Repo anlegen“ aktiviert ist.
- Weitere Ausschlüsse: `--exclude-caches`, Markerdateien (`--exclude-if-present`, z. B. `.nobackup`), Größenlimit (`--exclude-larger-than`) und Vorlage „Übliche Ausschlüsse“.
- Button „Repository prüfen (check)“ im Snapshots-Tab.
- Protokoll des letzten Laufs (`last-run.log`), auch für den Zeitplan; Anzeige im Backup-Tab.
- Warnung im Log, wenn restic Dateien nicht lesen konnte (Exit-Code 3).
- Versionsanzeige in Fenstertitel und Tab-Leiste, App-Icon.
- Eingabeprüfung (Uhrzeit, Aufbewahrungsregeln) vor Speichern und Backup.

### Behoben
- Repo-Zustand wird unterschieden (nicht vorhanden / falsches Passwort / Fehler) statt jeden Fehler als „kein Repo“ zu werten.
- `forget` räumt nur noch Snapshots des eigenen Rechners auf (wichtig bei geteilten Repos).
- Keine parallelen restic-Aufrufe mehr in der GUI.
- Freigabe wird beim Beenden nur getrennt, wenn die App sie selbst verbunden hat; laufende Jobs werden vorher abgebrochen.
- Windows: SMB-Passwort wird nicht mehr als Kommandozeilenargument übergeben (Fallback vorhanden); Fehler 1219 wird korrekt erkannt; kein Konsolenfenster beim Zeitplan-Lauf.
- Linux: doppelten Credentials-Code entfernt.

## [0.1.0]
- Erste Version: restic-Backup-App (egui) mit SMB-Mount, Zeitplan, Restore und Retention.

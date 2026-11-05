# Changelog

## [Unveröffentlicht]

### Neu
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

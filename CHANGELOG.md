# Changelog

## [0.3.20]

### Behoben
- Windows-Installer: Läuft noch eine `restic.exe` aus dem Installationsordner (z. B. ein Backup, dessen App beendet wurde), beendet der Installer sie jetzt mit. Vorher brach das Update mit „DeleteFile schlug fehl; Code 5. Zugriff verweigert“ ab. Andere restic-Programme des Benutzers bleiben unberührt.

## [0.3.19]

### Geändert
- Die App lässt sich nicht mehr zweimal öffnen (Windows und Linux). Ein zweiter Start holt das laufende Fenster nach vorn, auch aus dem Tray, und beendet sich. Der Start beim Anmelden (`--autostart`) und der Zeitplan sind davon nicht betroffen.

### Behoben
- Linux: Das Fenster-Icon (Titelleiste, Taskleiste, Alt-Tab) fehlte, weil das Fenster keine feste Kennung hatte und das System es nicht der `.desktop`-Datei zuordnen konnte. Das Fenster meldet sich jetzt als `restic-backup-app` (`app_id`, `StartupWMClass`).
- Das Protokoll nennt jetzt, welche Dateien bei einer Warnung nicht gesichert werden konnten („Nicht lesbar: …“, die ersten 10 je Ordner). Bisher stand dort nur „Backup mit Warnungen“.
- Windows: Desktop-Meldungen erscheinen als „restic Backup“ statt als „Windows PowerShell“.

## [0.3.18]

### Hinzugefügt
- Snapshots-Tab: Dateisuche im gewählten Snapshot (`restic find`, ohne Beachtung der Groß-/Kleinschreibung, `*` und `?` möglich). Treffer lassen sich einzeln wiederherstellen oder (Ordner) in der Ordneransicht öffnen.
- Neue Option „Auch nach erfolgreichem Backup eine Desktop-Meldung senden“ (Info-Tab, Standard aus). Warnungen und Fehler werden weiterhin immer gemeldet.

### Geändert
- Linux: Neue Option „Nicht in andere Laufwerke und Netzfreigaben wechseln“ (`--one-file-system`, Standard an). Eingehängte Laufwerke, USB-Platten und Netzfreigaben unterhalb eines gewählten Ordners werden nicht mehr mitgesichert, nur die gewählten Ordner selbst.
- Die Oberfläche zeichnet nur noch alle 250 ms neu, solange etwas läuft, statt bei jedem Frame (weniger CPU- und GPU-Last).
- Grafik über OpenGL (glow) statt wgpu/Vulkan: schnellerer Start, weniger Speicher. Die Oberfläche braucht kein Vulkan.

### Behoben
- Das Datenverzeichnis der App (NAS-Mountpunkt mit dem Repository, restic-Cache, SMB-Zugangsdaten) wird immer vom Backup ausgeschlossen. Unter Linux liegt der Mountpunkt unter `~/.local/share`, ein gewähltes Home-Verzeichnis hätte also das Repository in sich selbst gesichert.
- Während eines Laufs drehten sich bis zu drei Spinner gleichzeitig. Es bleibt eine Anzeige „Arbeitet …“ in der Seitenleiste.

## [0.3.17]

### Hinzugefügt
- Snapshots-Tab: „Sperren entfernen“ führt `restic unlock` aus (nur veraltete Sperren). Mit „auch aktive“ werden alle Sperren entfernt (`--remove-all`). Läuft auf diesem Rechner gerade ein Backup, wird nichts entfernt.
- Einstellungen: Feld „Rechnername“ für Snapshots. Ein fester Name verhindert, dass nach einer Umbenennung des Rechners das ganze Backup neu eingelesen wird (kein Parent-Snapshot) und das Aufräumen alte Snapshots übersieht. Leer = Name des Rechners wie bisher.

### Geändert
- Neue Einstellung „Schneller sichern“ (Standard an): restic zählt die Dateien nicht mehr vorab (`--no-scan`), das Backup beginnt sofort. Dafür zeigt die Fortschrittsanzeige Dateien und Datenmenge statt Prozent.
- Backups auf das NAS sind schneller: größere Packdateien (`--pack-size 64`, weniger SMB-Roundtrips) und mehr parallele Schreibvorgänge im Repository (`-o local.connections=8`, ab restic 0.17). Die App prüft die restic-Version und nutzt nur, was sie kennt; ältere Versionen laufen unverändert.
- Linux-Build: linuxdeploy und die AppImage-Runtime werden mit Wiederholungen geladen (GitHub lieferte gelegentlich HTTP 504).

### Behoben
- Ein GUI-Backup, das wegen eines bereits laufenden Backups nicht startete, zählte als Fehlschlag und hielt den Zeitplan 2 Stunden zurück.
- Liegt der letzte Erfolg in der Zukunft (Uhr zurückgestellt, Zeitzonenwechsel), blockiert das den Zeitplan nicht mehr.
- Ein hängengebliebenes Repo-Lock führt im Protokoll zu einem Hinweis auf „Sperren entfernen“.
- Zeitstempel der Repository-Prüfung und des Aufräumens werden atomar geschrieben.

## [0.3.16]

### Geändert
- „Übliche Ausschlüsse“ unterscheidet jetzt zwischen Windows und Linux und wählt automatisch die Muster für das System, auf dem die App läuft (plus allgemeine Muster wie `node_modules`). Der Button zeigt das System an, z. B. „Übliche Ausschlüsse (Windows)“.

## [0.3.15]

### Geändert
- Windows: Die App setzt `DISABLE_RTSS_LAYER=1` für sich selbst. Dadurch lädt RTSS seinen Vulkan-Layer nicht mehr in die App, und das FPS-Overlay von MSI Afterburner wird nicht mehr eingeblendet.

## [0.3.14]

### Geändert
- Windows-Installer: Eine laufende Instanz der App (Fenster, Tray oder Zeitplan-Lauf) wird vor dem Update bzw. der Deinstallation automatisch beendet.
- „Übliche Ausschlüsse“ ist deutlich erweitert (Windows und Linux): weitere Browser-, Electron-, GPU- und Shader-Caches (NVIDIA, Brave, Edge, Chrome, Firefox, Discord, VS Code), Windows-Update- und WebCache-Ordner, `AppData\Local\Programs`, Paket-Caches (pip, NuGet, Maven, rustup), Spiele-Daten (Fortnite, Steam) sowie unter Linux Flatpak, Container, Steam und Systemordner wie `/proc` und `/tmp`. So wird weniger Unnötiges gesichert, und das Backup dauert kürzer.

## [0.3.13]

### Behoben
- Windows: Die SMB-Verbindung scheiterte seit 0.3.11 mit „Systemfehler 67 – Der Netzwerkname wurde nicht gefunden“, weil dem UNC-Pfad ein führender Backslash fehlte (nur einer statt zwei vor dem Servernamen).

### Geändert
- Der Fortschrittsbalken ist nicht mehr animiert und zeichnet das Fenster dadurch nicht mehr dauernd neu (weniger GPU-Last).

## [0.3.12]

### Geändert
- Backups laufen deutlich schneller: Das Aufräumen alter Snapshots (`forget`) läuft weiter bei jedem Lauf, das langsame `--prune` (liest über SMB den ganzen Index) aber nur noch alle 7 Tage und mit `--max-unused 10%`.
- Die Teilprüfung (`restic check`) nutzt jetzt den lokalen Cache und muss die Metadaten nicht mehr jedes Mal neu über SMB laden.
- restic nutzt einen festen Cache im Datenverzeichnis (`restic-cache`), den GUI und Zeitplan gemeinsam verwenden.
- Das Backup liest mit `--read-concurrency 4` mehr Dateien parallel ein.
- Neue Einstellungen starten mit „Cache-Ordner ausschließen“ (`--exclude-caches`) aktiv.

### Behoben
- Ist der Rechnername unbekannt, wird das Aufräumen übersprungen statt die Snapshots anderer Rechner im Repo auszudünnen.

## [0.3.11]

### Geändert
- Windows: Besteht schon eine Anmeldung mit einem anderen Benutzer am selben NAS (z. B. Netzlaufwerke), verbindet die App die Backup-Freigabe unter einem zweiten Servernamen (Reverse-DNS-Name bzw. IP), den Windows als eigenen Server behandelt. Netzlaufwerke mit anderen Benutzern und das Backup stören sich so nicht mehr.

## [0.3.10]

### Behoben
- Der Zeitplan wird beim Start der App automatisch erneuert, wenn er mit einer älteren Programmversion angelegt wurde (vorher lief nach einem Update der alte Task mit `--run-backup` und nur den Uhrzeit-Triggern weiter, bis man „Zeitplan aktivieren“ erneut drückte).
- Windows: Ein „getrennter“ Netzwerk-Eintrag mit falschen Anmeldedaten wurde als verbunden gewertet, sodass erst restic nach rund 15 Minuten mit „Benutzername oder Kennwort falsch“ scheiterte. Jetzt gibt es einen echten Zugriffstest, veraltete Einträge werden ersetzt, und bei einer Anmeldung mit anderem Benutzer (Fehler 1219) kommt eine verständliche Meldung.
- Nach einem fehlgeschlagenen Zeitplan-Lauf wartet der Zeitplan zwei Stunden, statt alle 30 Minuten erneut zu sichern, zu melden und den Verlauf zu füllen.

## [0.3.9]

### Behoben
- Scheitert nach einem erfolgreichen Backup nur das Aufräumen alter Snapshots (z. B. Repo gesperrt), gilt der Lauf jetzt als Warnung statt als Fehler; vorher sicherte der Zeitplan deshalb alle 30 Minuten erneut.
- Backup konnte hängen bleiben, wenn restic eine Zeile mit ungültiger UTF-8-Kodierung ausgab.
- Ein Lauf ohne NAS-Adresse, Freigabe oder Quellordner endet mit einer klaren Meldung statt einer leeren Fehlermeldung.
- Einstellungen und Zustandsdateien (letzter Erfolg, Pause …) werden atomar geschrieben und bleiben bei Absturz oder Stromausfall heil.

### Geändert
- Release-Workflow vereinfacht: keine Eingaben mehr, die Version kommt aus der `Cargo.toml` von `main`; Changelog-Abschnitt und Commit-SHA sind nicht mehr nötig (`Cargo.lock` muss weiter zur Version passen).
- Release-Ablauf wie beim cctv-viewer: Die Version wird im Pull Request erhöht; der manuell gestartete Workflow `release` prüft Version, Changelog und den Build des Commits, veröffentlicht die dort gebauten Pakete unverändert (mit `SHA256SUMS.txt`) und ändert nichts an `main`. `build.yml` veröffentlicht nicht mehr selbst.

## [0.3.8]

### Neu
- Zeitplan für Laptops: Der Zeitplan startet das Programm jetzt zu den Uhrzeiten, bei der Anmeldung, beim Entsperren und alle 30 Minuten (Linux: Timer beim Start der Sitzung und alle 30 Minuten auf der Echtzeituhr, auch nach dem Aufwachen). Gesichert wird nur, wenn seit dem letzten geplanten Termin noch kein Backup gelang – kein doppeltes Sichern mehr nach langer Pause oder kurz nach einem manuellen Lauf.
- Ist das NAS nicht erreichbar (z. B. unterwegs), wird der Zeitplan-Lauf still übersprungen: kein Fehler, keine Meldung, `last-run.log` und Verlauf bleiben unverändert. Der nächste Trigger versucht es erneut.
- Meldung „Backup überfällig“, wenn seit einstellbar vielen Tagen (Standard 3, 0 = aus) kein Backup gelang; höchstens einmal täglich.
- Option „Nur am Netzteil sichern“ (Windows: Task-Einstellung, Linux: `ConditionACPower`). Windows startet den Task außerdem nur bei vorhandener Netzwerkverbindung.
- Zeitplan pausieren (bis morgen früh oder eine Woche) im Zeitplan-Tab und im Tray-Menü; Anzeige im Backup-Tab.
- Ein Backup läuft nie doppelt (GUI und Zeitplan sperren sich gegenseitig, die Sperre wird bei Absturz oder Stromausfall automatisch frei).
- Update-Suche: Info-Tab „Nach Updates suchen“; beim Start höchstens einmal täglich (abschaltbar). Es wird nur die öffentliche GitHub-Release-Seite abgefragt.

### Geändert
- Der Zeitplan startet `--scheduled` statt `--run-backup` (`--run-backup` sichert weiterhin immer). **Nach dem Update den Zeitplan einmal neu aktivieren.**
- Verbindungsversuche im Zeitplan-Lauf: 2 × 30 s statt 5 × 60 s, dafür vorher ein kurzer Erreichbarkeitstest.
- Windows: Zeitplan entfernen braucht nur noch einen PowerShell-Aufruf statt 25 `schtasks`-Prozessen.

## [0.3.7]

- Release 0.3.7

## [0.3.6]

- Release 0.3.6

## [0.3.5]

### Neu
- Lizenzdatei (MIT) hinzugefügt.
- Übersicht: Karte „Repository“ mit Größe, Snapshot-Anzahl und (unter Linux) freiem Platz auf dem NAS; die Daten werden per Knopfdruck geladen.
- Snapshots: Filter nach Ordner und Zeitraum (zusätzlich zum Textfilter) und Vergleich zweier Snapshots (`restic diff`).
- Einstellungen lassen sich exportieren und importieren (Info-Tab, ohne Passwörter).
- Regelmäßige Integritätsprüfung: Der Zeitplan-Lauf prüft alle 7 Tage 5 % der Repo-Daten (einstellbar, 0 = aus); ein Fehler zählt als Warnung. Die Übersicht zeigt das Datum der letzten Prüfung.
- Desktop-Benachrichtigung bei Warnung oder Fehler nach einem Lauf (auch bei Zeitplan-Läufen).
- Tray-Icon (Öffnen, Backup jetzt, Beenden): Fenster schließen legt es in den Tray. Ohne Tray-Dienst (z. B. GNOME ohne AppIndicator, unter Arch optional `libayatana-appindicator`) verhält sich die App wie bisher.

### Geändert
- Lange Hinweistexte sind unter „ⓘ Mehr“ aufklappbar; Einstellungen und Zeitplan sind gesperrt, solange ein Lauf aktiv ist.
- Release-Ablauf: Der Release-Workflow öffnet jetzt einen Release-PR statt direkt auf `main` zu pushen; `main` ist per PR und Pflichtprüfung „Anwendung prüfen“ geschützt.

### Behoben
- Übersicht: Doppelte Anzeigen entfernt – der aktuelle Schritt steht nur noch als Titel der Laufkarte (nicht zusätzlich als Zeile), und „Verlauf“ („Frühere Läufe“) wiederholt nicht mehr den Lauf aus „Letzter Lauf“.

## [0.3.4]

- Release 0.3.4

## [0.3.3]

### Neu
- Quellordner: Option „Jeden Ordner als eigenen Snapshot sichern“ (z. B. je ein Snapshot für `C:\Users\admin` und `C:\Users\xyz`). Die Aufbewahrung wird dann je Ordner angewendet; schlägt ein Ordner fehl, werden die übrigen trotzdem gesichert (Lauf gilt als Warnung). Beim Umstellen bleiben bereits vorhandene gemeinsame Snapshots bestehen und müssen bei Bedarf manuell gelöscht werden.

### Behoben
- Windows: Das Programmsymbol ist jetzt in der .exe eingebettet (vorher zeigten Explorer, Taskleiste und Verknüpfungen das Standard-Symbol).

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

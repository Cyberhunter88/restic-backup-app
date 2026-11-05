# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Rust-Desktop-App (eframe/egui) für restic-Backups auf ein NAS per SMB, Windows und Linux. Sprache der UI, Kommentare, Changelog und Commits ist Deutsch.

## Befehle
- `cargo build --release`
- `cargo test` (einzelner Test: `cargo test <name>`); die Integrationstests brauchen `restic` im PATH
- Linux-Build-Abhängigkeiten: `libdbus-1-dev pkg-config libxkbcommon-dev libwayland-dev libgtk-3-dev`; das Tray-Icon lädt `libayatana-appindicator` erst zur Laufzeit (optional, ohne sie gibt es keinen Tray)
- Headless-Lauf wie vom Zeitplan: `restic-backup-app --run-backup`; `--autostart` startet das Fenster minimiert
- Kein Linter konfiguriert; `#[cfg(windows)]`-Code lässt sich unter Linux nicht kompilieren oder testen, er wird erst in der Windows-CI geprüft

## Workflow
- Repo ist öffentlich, Lizenz MIT. `main` ist geschützt wie bei den anderen Projekten (z. B. cctv-viewer): nur per Pull Request, Pflichtprüfung „Anwendung prüfen“ (Job `pruefen` in `build.yml`, wartet auf Linux- und Windows-Build), Branch muss aktuell sein, Kommentare gelöst, Admins eingeschlossen, kein Force-Push. Also nie direkt auf `main` arbeiten: eigener Branch + PR, selbst mergen (keine Freigabe nötig). Der lokale `pre-push`-Hook (`git config core.hooksPath .githooks`) blockiert Pushes auf `main` zusätzlich.
- Pull Requests werden immer gebaut (auch reine Doku-Änderungen), sonst fehlt die Pflichtprüfung.
- Änderungen im `CHANGELOG.md` unter `## [Unveröffentlicht]` sammeln (neue Einträge nie unter einen schon veröffentlichten Abschnitt setzen).
- Release: Workflow `release` manuell in GitHub Actions starten (Branch `main`, Version ohne `v`). Er setzt Version in `Cargo.toml`/`Cargo.lock`, benennt den Changelog-Abschnitt um, pusht `release/vX.Y.Z`, öffnet den Release-PR und startet `build.yml` dafür (PRs mit dem Workflow-Token lösen selbst keine Workflows aus). Nach dem Mergen baut `build.yml` `main`, und der Job `release` legt Tag + GitHub-Release an, wenn es die Version aus `Cargo.toml` noch nicht gibt. Version also nie von Hand erhöhen.
- Voraussetzung: Repo-Einstellung „Allow GitHub Actions to create and approve pull requests“ (Settings → Actions → General). Ohne sie scheitert „Release-PR öffnen“; dann den PR von Hand anlegen (`gh pr create --head release/vX.Y.Z`) und `gh workflow run build.yml --ref release/vX.Y.Z` starten.

## Architektur
- `main.rs`: zwei Modi – GUI (`ui`/`app`) oder `--run-backup` (headless, schreibt via `config::write_last_run` nach `last-run.log` und in `history.tsv`). Der Zeitplan startet immer diese Headless-Variante der eigenen Binärdatei.
- `job.rs`: orchestriert einen Lauf: Mount (`smb`) → ggf. `restic init` → Backup → `forget --prune` → (nur im Zeitplan-Lauf, wenn fällig) Teilprüfung → Unmount (nur wenn die App selbst gemountet hat). `open_with_retry` wiederholt den Mount für Zeitplan-Läufe kurz nach dem Start.
- `restic.rs`: Wrapper um die restic-CLI (Argumente per `RESTIC_REPOSITORY`/`RESTIC_PASSWORD` als Env, `--json`-Ausgabe geparst). `stats`, `diff` und `check_subset` sind eigene Aufrufe. Backup-Exit-Code 3 bzw. ein Fehlercode nach geschriebenem Snapshot (Summary erhalten) zählt als Warnung, nicht als Fehler.
- `smb.rs`: plattformspezifisch – Windows UNC-Pfad, Linux `mount.cifs` (per pkexec bzw. fstab-Eintrag).
- `schedule.rs`: Linux systemd-User-Timer (`Persistent=true`), Windows Task per PowerShell-Cmdlets (ein Trigger je Uhrzeit, `StartWhenAvailable`; `schtasks` reicht dafür nicht). Mehrere Trigger müssen einzeln geklammert werden: `@((New-… ),(New-… ))`.
- `config.rs` / `secrets.rs`: Einstellungen als Datei im Datenverzeichnis (`directories`), Passwörter ausschließlich im System-Schlüsselbund.
- `app.rs` (GUI-Zustand, Tabs) führt lange Aufgaben in Threads aus und kommuniziert über `mpsc`-Kanäle mit der UI; `ui.rs` enthält Theme, Widgets (u. a. `hint_long`) und die testbare Filterlogik `snapshot_matches`. Einstellungen und Zeitplan sind gesperrt, solange `busy` ist. `config.rs` kann Einstellungen ohne Passwörter exportieren/importieren.
- `notify.rs` (Desktop-Meldung nur bei Warnung/Fehler) und `tray.rs` (Tray-Icon; Linux in eigenem GTK-Thread, ohne Tray-Dienst `None` → normales Beenden). `app.rs` verarbeitet Thread-Meldungen und Tray in `logic`, damit es auch bei verstecktem Fenster läuft.
- Teilprüfung: alle `check_interval_days` per `restic check --read-data-subset` (`config::check_due`, Ergebnis in `last-check.txt`); ein Fehler ist nur eine Warnung. `job::repo_overview` liefert Größe/Snapshots/freien Platz (`smb::free_space`) für die Repository-Karte.
- `autostart.rs`: Linux `~/.config/autostart`, Windows Run-Schlüssel. `update.rs` öffnet nur die Releases-Seite.
- Packaging in `packaging/` (Inno-Setup für Windows, `.desktop`, Arch-`PKGBUILD`/`install.sh`); Bau und Release in `.github/workflows/` (`build.yml` baut PRs und `main` und veröffentlicht; `release.yml` bereitet nur den Release-PR vor).

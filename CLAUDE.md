# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Rust-Desktop-App (eframe/egui) für restic-Backups auf ein NAS per SMB, Windows und Linux. Sprache der UI, Kommentare, Changelog und Commits ist Deutsch.

## Befehle
- `cargo build --release`
- `cargo test` (einzelner Test: `cargo test <name>`); die Integrationstests brauchen `restic` im PATH
- Linux-Build-Abhängigkeiten: `libdbus-1-dev pkg-config libxkbcommon-dev libwayland-dev libgtk-3-dev`
- Headless-Lauf wie vom Zeitplan: `restic-backup-app --run-backup`; `--autostart` startet das Fenster minimiert
- Kein Linter konfiguriert; `#[cfg(windows)]`-Code lässt sich unter Linux nicht kompilieren oder testen, er wird erst in der Windows-CI geprüft

## Workflow
- Nie direkt auf `main` arbeiten: immer eigener Branch + Pull Request (`git config core.hooksPath .githooks` aktiviert den `pre-push`-Hook, der Pushes auf `main` blockiert).
- Änderungen im `CHANGELOG.md` unter `## [Unveröffentlicht]` sammeln. Der Release-Workflow (manuell in GitHub Actions) setzt Version in `Cargo.toml`/`Cargo.lock` und benennt diesen Abschnitt um – Version also nicht von Hand erhöhen.

## Architektur
- `main.rs`: zwei Modi – GUI (`ui`/`app`) oder `--run-backup` (headless, schreibt via `config::write_last_run` nach `last-run.log` und in `history.tsv`). Der Zeitplan startet immer diese Headless-Variante der eigenen Binärdatei.
- `job.rs`: orchestriert einen Lauf: Mount (`smb`) → ggf. `restic init` → Backup → `forget --prune` → Unmount (nur wenn die App selbst gemountet hat). `open_with_retry` wiederholt den Mount für Zeitplan-Läufe kurz nach dem Start.
- `restic.rs`: Wrapper um die restic-CLI (Argumente per `RESTIC_REPOSITORY`/`RESTIC_PASSWORD` als Env, `--json`-Ausgabe geparst). Backup-Exit-Code 3 bzw. ein Fehlercode nach geschriebenem Snapshot (Summary erhalten) zählt als Warnung, nicht als Fehler.
- `smb.rs`: plattformspezifisch – Windows UNC-Pfad, Linux `mount.cifs` (per pkexec bzw. fstab-Eintrag).
- `schedule.rs`: Linux systemd-User-Timer (`Persistent=true`), Windows Task per PowerShell-Cmdlets (ein Trigger je Uhrzeit, `StartWhenAvailable`; `schtasks` reicht dafür nicht). Mehrere Trigger müssen einzeln geklammert werden: `@((New-… ),(New-… ))`.
- `config.rs` / `secrets.rs`: Einstellungen als Datei im Datenverzeichnis (`directories`), Passwörter ausschließlich im System-Schlüsselbund.
- `app.rs` (GUI-Zustand, Tabs) führt lange Aufgaben in Threads aus und kommuniziert über `mpsc`-Kanäle mit der UI; `ui.rs` enthält Theme und Widgets.
- `notify.rs` (Desktop-Meldung nur bei Warnung/Fehler) und `tray.rs` (Tray-Icon; Linux in eigenem GTK-Thread, ohne Tray-Dienst `None` → normales Beenden). `app.rs` verarbeitet Thread-Meldungen und Tray in `logic`, damit es auch bei verstecktem Fenster läuft.
- `job.rs` führt im Zeitplan-Lauf zusätzlich alle `check_interval_days` eine Teilprüfung aus (`config::check_due`, Datei `last-check.txt`).
- `autostart.rs`: Linux `~/.config/autostart`, Windows Run-Schlüssel. `update.rs` öffnet nur die Releases-Seite.
- Packaging in `packaging/` (Inno-Setup für Windows, `.desktop`, Arch-`PKGBUILD`/`install.sh`); Bau und Release in `.github/workflows/` (`build.yml` wird auch vom `release.yml` aufgerufen).

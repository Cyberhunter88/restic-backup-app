#!/bin/sh
# Installiert oder aktualisiert restic-backup-app unter Arch Linux.
# Erstinstallation:  git clone <Repo> && cd restic-backup-app && packaging/arch/install.sh
# Update:            packaging/arch/install.sh   (holt den neuesten Stand und baut das Paket neu)
set -e
cd "$(dirname "$0")"
root="$(git rev-parse --show-toplevel)"

if [ "$1" != "--no-pull" ]; then
  echo "==> Hole neuesten Stand (git pull) …"
  git -C "$root" pull --ff-only
fi

echo "==> Baue und installiere Paket (sudo-Passwort wird für pacman gebraucht) …"
makepkg -sif --needed
echo "==> Fertig. Installierte Version: $(pacman -Q restic-backup-app)"

#!/usr/bin/env bash
# Quita lo que instaló instalar-linux.sh (tu sesión guardada no se toca).
set -euo pipefail
rm -f "$HOME/.local/bin/appgrafica" \
      "$HOME/.local/share/applications/appgrafica.desktop" \
      "$HOME/.local/share/icons/hicolor/256x256/apps/appgrafica.png"
command -v update-desktop-database >/dev/null && update-desktop-database "$HOME/.local/share/applications" || true
echo "AppGrafica desinstalada."

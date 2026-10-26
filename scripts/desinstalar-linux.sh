#!/usr/bin/env bash
# Quita lo que instaló instalar-linux.sh (tu sesión guardada y tus proyectos no se tocan).
set -euo pipefail
rm -f "$HOME/.local/bin/appgrafica" \
      "$HOME/.local/share/applications/appgrafica.desktop" \
      "$HOME/.local/share/icons/hicolor/256x256/apps/appgrafica.png" \
      "$HOME/.local/share/mime/packages/appgrafica.xml"
command -v update-mime-database >/dev/null && update-mime-database "$HOME/.local/share/mime" || true
command -v update-desktop-database >/dev/null && update-desktop-database "$HOME/.local/share/applications" || true
echo "AppGrafica desinstalada."

#!/usr/bin/env bash
# Compila AppGrafica en modo optimizado y la instala para el usuario actual (sin sudo):
#   - el comando `appgrafica` en ~/.local/bin
#   - el icono y el acceso en el menú de aplicaciones
# Para actualizarla tras cambiar el código, basta con volver a ejecutar este script.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "Compilando AppGrafica (la primera vez tarda unos minutos)…"
cargo build --release

bin="$HOME/.local/bin"
apps="$HOME/.local/share/applications"
icons="$HOME/.local/share/icons/hicolor/256x256/apps"
mkdir -p "$bin" "$apps" "$icons"

install -m755 target/release/app_grafica "$bin/appgrafica"
install -m644 assets/icon.png "$icons/appgrafica.png"
cat > "$apps/appgrafica.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=AppGrafica
GenericName=Calculadora gráfica
Comment=Calculadora gráfica: funciones, ecuaciones, desigualdades y más
Exec=$bin/appgrafica %f
MimeType=application/x-appgrafica;
Icon=appgrafica
Terminal=false
Categories=Education;Math;Science;
Keywords=graficar;calculadora;matemáticas;funciones;gráfica;
StartupWMClass=appgrafica
DESKTOP

# Tipo de archivo de los proyectos (.agraf): se abren con doble clic.
mime="$HOME/.local/share/mime"
mkdir -p "$mime/packages"
cat > "$mime/packages/appgrafica.xml" <<MIME
<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="application/x-appgrafica">
    <comment>Proyecto de AppGrafica</comment>
    <glob pattern="*.agraf"/>
    <icon name="appgrafica"/>
  </mime-type>
</mime-info>
MIME
command -v update-mime-database >/dev/null && update-mime-database "$mime" || true

command -v update-desktop-database >/dev/null && update-desktop-database "$apps" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "Listo. Ábrela desde el menú de aplicaciones (\"AppGrafica\") o con el comando: appgrafica"

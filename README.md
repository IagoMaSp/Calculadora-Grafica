# AppGrafica

Calculadora gráfica de escritorio (estilo Desmos/GeoGebra) escrita en Rust con
[egui](https://github.com/emilk/egui). Funciona en Linux y en Windows.

- Funciones, ecuaciones implícitas, desigualdades, restricciones `{x > 0}`.
- Paramétricas, polares, puntos (arrastrables), listas.
- Deslizadores animados, números complejos, sumas, integrales y derivadas.
- Escritura matemática y notación LaTeX (`\frac{x}{2}`), teclado en pantalla.
- Textos sobre la gráfica para explicar cosas, con valores que se actualizan (`a = {a}`).
- Proyectos: guardar y abrir trabajos en archivos `.agraf`; exportar la gráfica como PNG
  o copiarla al portapapeles.
- Guía de uso integrada (botón **?** o tecla F1).

## Linux

Requiere [Rust](https://rustup.rs). Para instalarla para tu usuario (comando
`appgrafica` y acceso en el menú de aplicaciones):

```bash
./scripts/instalar-linux.sh
```

Vuelve a ejecutarlo para actualizarla tras cambiar el código. Para quitarla:
`./scripts/desinstalar-linux.sh`.

Para desarrollar: `cargo run` (o `cargo run -- "y = x^2"` para abrirla con
expresiones; en ese caso no se guarda la sesión al cerrar).

## Windows

Cada push compila un `AppGrafica.exe` en GitHub Actions (pestaña **Actions** →
la última ejecución → *Artifacts* → **AppGrafica-Windows**). Al subir una
etiqueta `v*` se publica además en **Releases**. Es un único `.exe`, no hace
falta instalar nada; ver `windows/LEEME.txt`.

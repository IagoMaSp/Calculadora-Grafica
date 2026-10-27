// En Windows, no abrir una consola junto a la ventana en modo release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod app;
mod editor;
mod help;
mod keyboard;
mod latex;
mod math;
mod plot;
mod points;
mod pretty;
mod project;
#[cfg(test)]
mod snapshot;
mod special;
mod style;

fn main() {
    // Expresiones iniciales opcionales: app_grafica "y = x^2" "x^2 + y^2 = 1"
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Primero wgpu (DirectX 12 / Vulkan / Metal); si la tarjeta gráfica no lo admite,
    // se reintenta con OpenGL, más antiguo pero disponible casi en cualquier equipo.
    // APPGRAFICA_RENDERER=glow fuerza OpenGL directamente.
    let force_glow = std::env::var("APPGRAFICA_RENDERER").is_ok_and(|v| v.eq_ignore_ascii_case("glow"));
    let first = if force_glow { Err(eframe::Error::AppCreation("OpenGL forzado".into())) } else { run(eframe::Renderer::Wgpu, args.clone()) };
    if let Err(e) = first {
        if let Err(e2) = run(eframe::Renderer::Glow, args) {
            show_fatal_error(&format!(
                "AppGrafica no pudo iniciar la parte gráfica.\n\nwgpu: {e}\nOpenGL: {e2}\n\n\
                 Prueba a actualizar los drivers de la tarjeta gráfica."
            ));
        }
    }
}

fn run(renderer: eframe::Renderer, args: Vec<String>) -> eframe::Result {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("AppGrafica")
        // Identificador de la ventana: la asocia a su acceso (.desktop) y a su icono.
        .with_app_id("appgrafica")
        .with_inner_size([1200.0, 760.0])
        .with_min_inner_size([600.0, 400.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, renderer, ..Default::default() };
    eframe::run_native("AppGrafica", options, Box::new(|cc| Ok(Box::new(app::GraphApp::new(cc, args)))))
}

/// Muestra un error aunque no haya consola (en Windows la app no abre ninguna).
fn show_fatal_error(msg: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (text, title) = (wide(msg), wide("AppGrafica"));
        // SAFETY: cadenas UTF-16 terminadas en 0 que viven hasta el final de la llamada.
        unsafe {
            MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
        }
    }
    eprintln!("{msg}");
}

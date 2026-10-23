// En Windows, no abrir una consola junto a la ventana en modo release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod app;
mod math;
mod plot;
mod style;

fn main() -> eframe::Result {
    // Expresiones iniciales opcionales: app_grafica "y = x^2" "x^2 + y^2 = 1"
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("AppGrafica")
            .with_inner_size([1200.0, 760.0])
            .with_min_inner_size([600.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("AppGrafica", options, Box::new(|cc| Ok(Box::new(app::GraphApp::new(cc, args)))))
}

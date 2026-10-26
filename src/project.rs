//! Archivos: proyectos (.agraf, JSON) y exportación de la gráfica como imagen PNG.

use std::path::{Path, PathBuf};

use eframe::egui::ColorImage;

/// Extensión de los proyectos.
pub const EXTENSION: &str = "agraf";

/// Máximo de proyectos recientes que se recuerdan.
pub const MAX_RECENT: usize = 8;

/// Diálogo "Abrir proyecto".
pub fn pick_open(dir: Option<&Path>) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new().set_title("Abrir proyecto").add_filter("Proyecto de AppGrafica", &[EXTENSION]);
    if let Some(dir) = dir {
        d = d.set_directory(dir);
    }
    d.pick_file()
}

/// Diálogo "Guardar proyecto"; añade la extensión si falta.
pub fn pick_save(dir: Option<&Path>, name: &str) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new()
        .set_title("Guardar proyecto")
        .add_filter("Proyecto de AppGrafica", &[EXTENSION])
        .set_file_name(format!("{name}.{EXTENSION}"));
    if let Some(dir) = dir {
        d = d.set_directory(dir);
    }
    d.save_file().map(|p| with_extension(p, EXTENSION))
}

/// Diálogo "Exportar imagen"; añade la extensión si falta.
pub fn pick_png(dir: Option<&Path>, name: &str) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new().set_title("Exportar imagen").add_filter("Imagen PNG", &["png"]).set_file_name(format!("{name}.png"));
    if let Some(dir) = dir {
        d = d.set_directory(dir);
    }
    d.save_file().map(|p| with_extension(p, "png"))
}

fn with_extension(p: PathBuf, ext: &str) -> PathBuf {
    if p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) { p } else { p.with_extension(ext) }
}

/// Nombre del archivo sin extensión ("parabolas.agraf" → "parabolas").
pub fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// ¿Parece un proyecto? (para abrirlo desde la línea de comandos o al soltarlo en la ventana).
pub fn is_project(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// Pone `p` el primero de la lista de recientes.
pub fn push_recent(recent: &mut Vec<PathBuf>, p: &Path) {
    recent.retain(|r| r != p);
    recent.insert(0, p.to_path_buf());
    recent.truncate(MAX_RECENT);
}

/// Guarda una imagen como PNG.
pub fn write_png(path: &Path, img: &ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let [w, h] = img.size;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&data).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(with_extension("a/b".into(), "png"), PathBuf::from("a/b.png"));
        assert_eq!(with_extension("a/b.PNG".into(), "png"), PathBuf::from("a/b.PNG"));
        assert_eq!(stem(Path::new("/x/parabolas.agraf")), "parabolas");
        assert!(is_project(Path::new("p.AGRAF")));
        let mut r = vec![PathBuf::from("a"), PathBuf::from("b")];
        push_recent(&mut r, Path::new("b"));
        assert_eq!(r, vec![PathBuf::from("b"), PathBuf::from("a")]);
    }

    #[test]
    fn png_roundtrip() {
        let img = ColorImage::new([3, 2], vec![eframe::egui::Color32::RED; 6]);
        let path = std::env::temp_dir().join("appgrafica-test.png");
        write_png(&path, &img).unwrap();
        let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let info = dec.read_info().unwrap().info().clone();
        assert_eq!((info.width, info.height), (3, 2));
        let _ = std::fs::remove_file(path);
    }
}

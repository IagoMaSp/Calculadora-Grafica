//! Solo para los tests: dibuja la interfaz sin ventana (rasterizador por software de los
//! triángulos de egui) y la guarda como PNG, para revisar el aspecto sin abrir la app.

use std::collections::HashMap;

use eframe::egui::{self, Color32, ColorImage, ImageData, Pos2, Rect, TextureId, epaint::Primitive, pos2, vec2};

/// Ejecuta `frames` fotogramas de `ui_fn` (con los eventos de cada uno) y devuelve la imagen
/// del último.
pub fn render(size: [f32; 2], ppp: f32, frames: Vec<Vec<egui::Event>>, mut ui_fn: impl FnMut(&mut egui::Ui)) -> ColorImage {
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(ppp);
    let mut textures: HashMap<TextureId, ColorImage> = HashMap::new();
    let mut last = None;
    for events in frames {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size.into())), events, ..Default::default() };
        let mut out = ctx.run_ui(input, &mut ui_fn);
        for (id, deltas) in &out.textures_delta.set {
            for d in deltas {
                let ImageData::Color(img) = &d.image;
                match d.pos {
                    None => {
                        textures.insert(*id, (**img).clone());
                    }
                    Some([x0, y0]) => {
                        if let Some(t) = textures.get_mut(id) {
                            for y in 0..img.size[1] {
                                for x in 0..img.size[0] {
                                    t.pixels[(y0 + y) * t.size[0] + x0 + x] = img.pixels[y * img.size[0] + x];
                                }
                            }
                        }
                    }
                }
            }
        }
        out.textures_delta.clear();
        last = Some(ctx.tessellate(out.shapes, out.pixels_per_point));
    }
    let [w, h] = [(size[0] * ppp) as usize, (size[1] * ppp) as usize];
    let mut img = ColorImage::filled([w, h], Color32::BLACK);
    for prim in last.unwrap_or_default() {
        let Primitive::Mesh(mesh) = prim.primitive else { continue };
        let clip = Rect::from_min_max(prim.clip_rect.min * ppp, prim.clip_rect.max * ppp).intersect(Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32)));
        let tex = textures.get(&mesh.texture_id);
        for tri in mesh.indices.chunks_exact(3) {
            let v = [&mesh.vertices[tri[0] as usize], &mesh.vertices[tri[1] as usize], &mesh.vertices[tri[2] as usize]];
            let p: Vec<Pos2> = v.iter().map(|v| pos2(v.pos.x * ppp, v.pos.y * ppp)).collect();
            let area = (p[1].x - p[0].x) * (p[2].y - p[0].y) - (p[2].x - p[0].x) * (p[1].y - p[0].y);
            if area.abs() < 1e-6 {
                continue;
            }
            let bb = Rect::from_points(&p).intersect(clip);
            if !bb.is_positive() {
                continue;
            }
            for y in bb.min.y.floor() as usize..(bb.max.y.ceil() as usize).min(h) {
                for x in bb.min.x.floor() as usize..(bb.max.x.ceil() as usize).min(w) {
                    let q = pos2(x as f32 + 0.5, y as f32 + 0.5);
                    if !clip.contains(q) {
                        continue;
                    }
                    let edge = |a: Pos2, b: Pos2| (b.x - a.x) * (q.y - a.y) - (q.x - a.x) * (b.y - a.y);
                    let (w0, w1, w2) = (edge(p[1], p[2]) / area, edge(p[2], p[0]) / area, edge(p[0], p[1]) / area);
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let mix = |f: &dyn Fn(usize) -> f32| w0 * f(0) + w1 * f(1) + w2 * f(2);
                    let mut c = [0.0f32; 4];
                    for (k, ch) in c.iter_mut().enumerate() {
                        *ch = mix(&|i| v[i].color.to_array()[k] as f32 / 255.0);
                    }
                    if let Some(t) = tex {
                        let (u, vv) = (mix(&|i| v[i].uv.x), mix(&|i| v[i].uv.y));
                        let tx = ((u * t.size[0] as f32) as usize).min(t.size[0] - 1);
                        let ty = ((vv * t.size[1] as f32) as usize).min(t.size[1] - 1);
                        let s = t.pixels[ty * t.size[0] + tx].to_array();
                        for k in 0..4 {
                            c[k] *= s[k] as f32 / 255.0;
                        }
                    }
                    // Mezcla con alfa premultiplicado.
                    let dst = &mut img.pixels[y * w + x];
                    let d = dst.to_array();
                    let a = c[3];
                    let out: Vec<u8> = (0..4).map(|k| ((c[k] + d[k] as f32 / 255.0 * (1.0 - a)) * 255.0).round().clamp(0.0, 255.0) as u8).collect();
                    *dst = Color32::from_rgba_premultiplied(out[0], out[1], out[2], 255);
                }
            }
        }
    }
    img
}

/// Carpeta donde dejar las imágenes (APPGRAFICA_SNAPSHOTS); si no está, no se guardan.
pub fn save(name: &str, img: &ColorImage) {
    if let Ok(dir) = std::env::var("APPGRAFICA_SNAPSHOTS") {
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        crate::project::write_png(&path, img).expect("no se pudo guardar la imagen");
    }
}

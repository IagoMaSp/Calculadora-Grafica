// Incrusta el icono y la información del programa en el .exe de Windows.
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "AppGrafica")
            .set("FileDescription", "AppGrafica - calculadora gráfica")
            .set("LegalCopyright", "IagoMaSp");
        if let Err(e) = res.compile() {
            println!("cargo:warning=no se pudo incrustar el icono: {e}");
        }
    }
}

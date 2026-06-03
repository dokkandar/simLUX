mod aci_picker;
mod app;
mod gpu;
mod settings;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_title("RUST_CAD — math calculation workbench"),
        ..Default::default()
    };
    eframe::run_native(
        "rust_cad",
        options,
        Box::new(|_cc| Ok(Box::new(app::CadApp::default()))),
    )
}

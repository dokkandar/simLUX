mod aci_picker;
mod app;
mod dbg_recorder;
mod gpu;
mod hatch_trace;
mod settings;
mod theme;
mod varreg;
// wall feature logic now lives in the `cad_wall` crate (see ARCHITECTURE.md).

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_title("AutoRASM 2026"),
        ..Default::default()
    };
    eframe::run_native(
        "rust_cad",
        options,
        Box::new(|_cc| Ok(Box::new(app::CadApp::default()))),
    )
}

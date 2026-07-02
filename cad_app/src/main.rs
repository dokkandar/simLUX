mod aci_picker;
mod app;
mod dbg_recorder;
mod dock;
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
        Box::new(|cc| {
            // Load Geist + JetBrains Mono before the first frame (THEME_SYSTEM §5.7).
            theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(app::CadApp::default()))
        }),
    )
}

mod aci_picker;
mod app;
mod command;
mod dbg_recorder;
mod dock;
mod gpu;
mod hatch_trace;
mod light;
mod light3d;
mod param_editor;
mod settings;
mod theme;
mod varreg;
// wall feature logic now lives in the `cad_wall` crate (see ARCHITECTURE.md).

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_title("SIMLUX — Lighting Designer"),
        ..Default::default()
    };
    eframe::run_native(
        "simlux",
        options,
        Box::new(|cc| {
            // Load Geist + JetBrains Mono before the first frame (THEME_SYSTEM §5.7).
            theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(app::CadApp::default()))
        }),
    )
}

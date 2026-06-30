use eframe::egui;
use image::GenericImageView;
use terminaste_settings::load_settings;
use terminaste_ui::TerminasteApp;

#[cfg(target_os = "macos")]
mod macos_menu;

fn main() -> eframe::Result {
    let loaded = load_settings();
    let options = eframe::NativeOptions {
        // Metal drawable acquisition blocks AppKit during off-screen window animations.
        renderer: if cfg!(target_os = "macos") {
            eframe::Renderer::Glow
        } else {
            eframe::Renderer::Wgpu
        },
        #[cfg(target_os = "macos")]
        vsync: false,
        hardware_acceleration: eframe::HardwareAcceleration::Required,
        persist_window: true,
        persistence_path: Some(loaded.path.with_file_name("window.ron")),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            desired_maximum_frame_latency: Some(1),
            ..Default::default()
        },
        viewport: egui::ViewportBuilder::default()
            .with_title("terminaste")
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([160.0, 160.0])
            .with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "terminaste",
        options,
        Box::new(move |_cc| {
            #[cfg(target_os = "macos")]
            let menu_events = macos_menu::install(_cc.egui_ctx.clone());
            let app = TerminasteApp::new(loaded);
            #[cfg(target_os = "macos")]
            let app = macos_menu::MacosApp(app, menu_events);
            Ok(Box::new(app))
        }),
    )
}

fn app_icon() -> egui::IconData {
    if cfg!(target_os = "macos")
        && std::env::current_exe().is_ok_and(|path| {
            path.parent()
                .filter(|parent| parent.ends_with("Contents/MacOS"))
                .and_then(|parent| parent.parent())
                .is_some_and(|contents| contents.join("Info.plist").is_file())
        })
    {
        // An empty icon keeps eframe from replacing the bundle's multi-resolution Dock icon.
        return egui::IconData::default();
    }
    let image = image::load_from_memory(include_bytes!("../../../assets/icon.png"))
        .expect("packaged app icon must be valid PNG");
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_rgba8().into_raw(),
        width,
        height,
    }
}

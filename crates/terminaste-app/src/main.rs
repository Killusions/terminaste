use std::borrow::Cow;

use gpui::{px, size, App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions};
use terminaste_settings::load_settings;
use terminaste_ui::{install_actions, TerminalWindow, TerminasteApp};

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![Cow::Borrowed(
                include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf").as_slice(),
            )])
            .expect("bundled terminal font must load");
        install_actions(cx);
        let app = TerminasteApp::new(load_settings());
        let bounds = app.window_bounds().unwrap_or_else(|| {
            WindowBounds::Windowed(Bounds::centered(None, size(px(1180.), px(760.)), cx))
        });
        cx.open_window(
            WindowOptions {
                window_bounds: Some(bounds),
                window_min_size: Some(size(px(160.), px(160.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("terminaste".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| TerminalWindow::new(app, window, cx)),
        )
        .expect("could not open terminal window");
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}

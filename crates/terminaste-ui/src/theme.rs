use gpui::{rgb, Hsla};
use terminaste_settings::{AppearanceMode, Settings};

#[derive(Clone)]
pub(super) struct TerminalTextStyle {
    pub font: gpui::Font,
    pub font_size: gpui::Pixels,
    pub cell: gpui::Size<gpui::Pixels>,
    pub theme: TerminasteTheme,
    pub drop_background_if_readable: bool,
}

#[derive(Clone, Copy)]
pub struct TerminasteTheme {
    pub background: Hsla,
    pub surface: Hsla,
    pub surface_high: Hsla,
    pub selection: Hsla,
    pub active_option: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub error: Hsla,
}

impl TerminasteTheme {
    pub fn light() -> Self {
        Self {
            background: rgb(0xf7f8fa).into(),
            surface: rgb(0xffffff).into(),
            surface_high: rgb(0xeff2f7).into(),
            selection: rgb(0xb8d5fa).into(),
            active_option: rgb(0xe8eff9).into(),
            text: rgb(0x121826).into(),
            muted: rgb(0x64748b).into(),
            border: rgb(0xdadfe8).into(),
            accent: rgb(0x64708a).into(),
            error: rgb(0xdc2626).into(),
        }
    }
    pub fn dark() -> Self {
        Self {
            background: rgb(0x0a0b0f).into(),
            surface: rgb(0x11131a).into(),
            surface_high: rgb(0x191c26).into(),
            selection: rgb(0x365780).into(),
            active_option: rgb(0x283b55).into(),
            text: rgb(0xeef2ff).into(),
            muted: rgb(0x94a3b8).into(),
            border: rgb(0x2a303e).into(),
            accent: rgb(0x707d96).into(),
            error: rgb(0xef4444).into(),
        }
    }

    pub fn terminal_light() -> Self {
        let mut theme = Self::light();
        theme.background = rgb(0xffffff).into();
        theme.surface = rgb(0xffffff).into();
        theme
    }
    pub(super) fn for_settings(settings: &Settings, appearance: gpui::WindowAppearance) -> Self {
        let dark = match settings.appearance.mode {
            AppearanceMode::Dark => true,
            AppearanceMode::Light => false,
            AppearanceMode::System => matches!(
                appearance,
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
            ),
        };
        if dark {
            Self::dark()
        } else {
            Self::light()
        }
    }
}

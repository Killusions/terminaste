use std::cell::OnceCell;
use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use muda::{
    accelerator::{Accelerator, Code, Modifiers},
    Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
};
use terminaste_ui::TerminasteApp;

pub struct MacosApp(pub TerminasteApp, pub Receiver<MenuEvent>);

impl eframe::App for MacosApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.0.save(storage);
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.0.update(ctx, frame);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        while let Ok(event) = self.1.try_recv() {
            match event.id.0.as_str() {
                "edit-cut" => input.events.push(egui::Event::Cut),
                "edit-copy" => input.events.push(egui::Event::Copy),
                "edit-select-all" => input.events.push(egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::MAC_CMD,
                }),
                _ => {}
            }
        }
    }
}

fn edit_item(id: &str, text: &str, key: Code) -> MenuItem {
    MenuItem::with_id(
        id,
        text,
        true,
        Some(Accelerator::new(Some(Modifiers::SUPER), key)),
    )
}

thread_local! {
    static APP_MENU: OnceCell<Menu> = const { OnceCell::new() };
}

pub fn install(ctx: egui::Context) -> Receiver<MenuEvent> {
    let (sender, receiver) = mpsc::channel();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id.0 == "edit-paste" {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        } else if sender.send(event).is_err() {
            return;
        }
        ctx.request_repaint();
    }));
    APP_MENU.with(|stored| {
        stored.get_or_init(|| {
            let menu = Menu::new();
            let application = Submenu::new("terminaste", true);
            application
                .append_items(&[
                    &PredefinedMenuItem::about(None, None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::services(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::hide(None),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::show_all(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::quit(None),
                ])
                .expect("failed to build application menu");

            let edit = Submenu::new("Edit", true);
            edit.append_items(&[
                &PredefinedMenuItem::undo(None),
                &PredefinedMenuItem::redo(None),
                &PredefinedMenuItem::separator(),
                &edit_item("edit-cut", "Cut", Code::KeyX),
                &edit_item("edit-copy", "Copy", Code::KeyC),
                &edit_item("edit-paste", "Paste", Code::KeyV),
                &edit_item("edit-select-all", "Select All", Code::KeyA),
            ])
            .expect("failed to build edit menu");

            let window = Submenu::new("Window", true);
            window
                .append_items(&[
                    &PredefinedMenuItem::minimize(None),
                    &PredefinedMenuItem::maximize(Some("Zoom")),
                    &PredefinedMenuItem::fullscreen(None),
                    &PredefinedMenuItem::separator(),
                    &PredefinedMenuItem::bring_all_to_front(None),
                ])
                .expect("failed to build window menu");

            menu.append_items(&[&application, &edit, &window])
                .expect("failed to build macOS menu bar");
            menu.init_for_nsapp();
            menu
        });
    });
    receiver
}

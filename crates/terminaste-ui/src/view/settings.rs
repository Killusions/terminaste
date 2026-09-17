use terminaste_settings::{save_settings, AppearanceMode, BlockSpacing, ClipboardEscapePolicy};

use super::*;

impl TerminalWindow {
    pub(super) fn render_settings(
        &self,
        viewport: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let narrow = viewport.width < px(520.);
        let mut navigation = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(6.))
            .w(px(130.))
            .flex_shrink_0();
        if narrow {
            navigation = navigation.w_full().flex_row().flex_wrap();
        }
        for (index, category) in SettingsCategory::ALL.into_iter().enumerate() {
            navigation = navigation.child(
                button(("settings-category", index), category.label(), theme)
                    .h(px(26.))
                    .px(px(8.))
                    .justify_start()
                    .when(!narrow, |item| item.w_full())
                    .when(self.category == category, |item| {
                        item.bg(theme.surface_high)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.category = category;
                        this.field = None;
                        cx.notify();
                    })),
            );
        }
        let content = div()
            .id("settings-content")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(12.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::MEDIUM)
                    .mb(px(12.))
                    .child(self.category.label()),
            )
            .child(self.settings_content(cx));
        let body = div()
            .flex()
            .flex_1()
            .min_h_0()
            .when(narrow, |body| body.flex_col())
            .child(navigation)
            .child(
                div()
                    .bg(theme.border)
                    .when(narrow, |line| line.h(px(1.)).w_full())
                    .when(!narrow, |line| line.w(px(1.)).h_full()),
            )
            .child(content);
        self.modal(
            "settings-modal",
            (viewport.width - px(16.)).min(px(720.)),
            cx,
        )
        .flex()
        .flex_col()
        .h((viewport.height - px(56.)).max(px(80.)))
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(34.))
                .flex_shrink_0()
                .px(px(12.))
                .border_b_1()
                .border_color(theme.border)
                .child(div().font_weight(FontWeight::MEDIUM).child("Settings"))
                .child(
                    button("close-settings", "×", theme)
                        .w(px(22.))
                        .text_size(px(16.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dismiss_overlay();
                            cx.notify();
                        })),
                ),
        )
        .child(body)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(40.))
                .px(px(10.))
                .flex_shrink_0()
                .border_t_1()
                .border_color(theme.border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_ellipsis()
                        .text_size(px(10.))
                        .text_color(theme.muted)
                        .child(self.app.loaded.path.display().to_string()),
                )
                .child(
                    button("save-settings", "Save settings", theme)
                        .px(px(10.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.surface_high)
                        .on_click(cx.listener(|this, _, _, cx| {
                            match save_settings(&this.app.loaded.path, &this.app.loaded.settings) {
                                Ok(()) => this.app.toast("Settings saved".to_owned()),
                                Err(error) => {
                                    this.app.toast(format!("Settings save failed: {error}"))
                                }
                            }
                            cx.notify();
                        })),
                ),
        )
    }

    fn settings_content(&self, cx: &mut Context<Self>) -> gpui::Div {
        let settings = &self.app.loaded.settings;
        let theme = self.theme;
        let mut content = div().flex().flex_col().gap(px(8.));
        match self.category {
            SettingsCategory::Appearance => {
                let mut choices = div().flex().gap(px(3.));
                for (index, (label, mode)) in [
                    ("System", AppearanceMode::System),
                    ("Light", AppearanceMode::Light),
                    ("Dark", AppearanceMode::Dark),
                ]
                .into_iter()
                .enumerate()
                {
                    choices = choices.child(
                        button(("theme", index), label, theme)
                            .px(px(8.))
                            .border_1()
                            .border_color(if settings.appearance.mode == mode {
                                theme.accent
                            } else {
                                theme.border
                            })
                            .when(settings.appearance.mode == mode, |item| {
                                item.bg(theme.surface_high)
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.app.loaded.settings.appearance.mode = mode;
                                cx.notify();
                            })),
                    );
                }
                content = content
                    .child(self.setting_row("Theme", choices))
                    .child(self.toggle(
                        "Minimum contrast",
                        settings.appearance.minimum_contrast,
                        |settings| {
                            settings.appearance.minimum_contrast =
                                !settings.appearance.minimum_contrast
                        },
                        cx,
                    ))
                    .child(self.toggle(
                        "Show zero-state blocks",
                        settings.appearance.zero_state_blocks,
                        |settings| {
                            settings.appearance.zero_state_blocks =
                                !settings.appearance.zero_state_blocks
                        },
                        cx,
                    ))
                    .child(self.stepper(
                        "Window opacity",
                        format!("{}%", (settings.appearance.window_opacity * 100.).round()),
                        |settings, delta| {
                            settings.appearance.window_opacity =
                                (settings.appearance.window_opacity + delta * 0.05).clamp(0.2, 1.)
                        },
                        cx,
                    ))
                    .child(
                        self.setting_row(
                            "Block spacing",
                            button(
                                "block-spacing",
                                format!("{:?}", settings.appearance.block_spacing),
                                theme,
                            )
                            .px(px(8.))
                            .border_1()
                            .border_color(theme.border)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.app.loaded.settings.appearance.block_spacing =
                                    match this.app.loaded.settings.appearance.block_spacing {
                                        BlockSpacing::Compact => BlockSpacing::Normal,
                                        BlockSpacing::Normal => BlockSpacing::Compact,
                                    };
                                cx.notify();
                            })),
                        ),
                    );
            }
            SettingsCategory::Fonts => {
                content = content
                    .child(div().text_color(theme.muted).child("Font family"))
                    .child(self.text_field(
                        "font-family",
                        Field::FontFamily,
                        &settings.font.family,
                        cx,
                    ))
                    .child(self.stepper(
                        "Terminal font size",
                        format!("{} px", settings.font.size),
                        |settings, delta| {
                            settings.font.size = (settings.font.size + delta).clamp(8., 48.)
                        },
                        cx,
                    ))
                    .child(self.stepper(
                        "Line height",
                        format!("{:.2}", settings.font.line_height),
                        |settings, delta| {
                            settings.font.line_height =
                                (settings.font.line_height + delta * 0.05).clamp(0.8, 2.2)
                        },
                        cx,
                    ))
                    .child(self.toggle(
                        "Ligatures",
                        settings.font.ligatures,
                        |settings| settings.font.ligatures = !settings.font.ligatures,
                        cx,
                    ));
            }
            SettingsCategory::Terminal => {
                content = content
                    .child(self.toggle(
                        "Audible bell",
                        settings.terminal.audible_bell,
                        |settings| settings.terminal.audible_bell = !settings.terminal.audible_bell,
                        cx,
                    ))
                    .child(self.stepper(
                        "Max grid rows",
                        settings.terminal.max_grid_rows.to_string(),
                        |settings, delta| {
                            settings.terminal.max_grid_rows =
                                (settings.terminal.max_grid_rows as f32 + delta * 1000.)
                                    .clamp(100., 1_000_000.)
                                    as usize
                        },
                        cx,
                    ))
                    .child(self.stepper(
                        "Alternate-screen padding",
                        settings.terminal.alternate_screen_padding.to_string(),
                        |settings, delta| {
                            settings.terminal.alternate_screen_padding =
                                (f32::from(settings.terminal.alternate_screen_padding) + delta)
                                    .clamp(0., 48.) as u16
                        },
                        cx,
                    ))
                    .child(
                        self.setting_row(
                            "Clipboard access",
                            button(
                                "clipboard-policy",
                                format!("{:?}", settings.terminal.clipboard_escape_policy),
                                theme,
                            )
                            .px(px(8.))
                            .border_1()
                            .border_color(theme.border)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.app.loaded.settings.terminal.clipboard_escape_policy =
                                    match this.app.loaded.settings.terminal.clipboard_escape_policy
                                    {
                                        ClipboardEscapePolicy::Deny => {
                                            ClipboardEscapePolicy::WriteOnly
                                        }
                                        ClipboardEscapePolicy::WriteOnly => {
                                            ClipboardEscapePolicy::ReadWrite
                                        }
                                        ClipboardEscapePolicy::ReadWrite => {
                                            ClipboardEscapePolicy::Deny
                                        }
                                    };
                                cx.notify();
                            })),
                        ),
                    );
            }
            SettingsCategory::Input => {
                content = content
                    .child(self.toggle(
                        "Left Alt as Meta",
                        settings.input.left_alt_is_meta,
                        |settings| {
                            settings.input.left_alt_is_meta = !settings.input.left_alt_is_meta
                        },
                        cx,
                    ))
                    .child(self.toggle(
                        "Right Alt as Meta",
                        settings.input.right_alt_is_meta,
                        |settings| {
                            settings.input.right_alt_is_meta = !settings.input.right_alt_is_meta
                        },
                        cx,
                    ))
                    .child(self.toggle(
                        "Vim-like editing",
                        settings.input.vim_like_editing,
                        |settings| {
                            settings.input.vim_like_editing = !settings.input.vim_like_editing
                        },
                        cx,
                    ));
            }
            SettingsCategory::Keybindings => {
                content = content
                    .child(self.text_field(
                        "keybinding-search",
                        Field::KeybindingSearch,
                        &self.keybinding_search,
                        cx,
                    ))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(theme.muted)
                            .child(format!(
                                "{} active bindings · {} conflicts",
                                self.app.loaded.keybindings.bindings.len(),
                                self.app.loaded.keybindings.conflicts.len()
                            )),
                    );
                for (action, shortcuts) in &self.app.loaded.keybindings.bindings {
                    let shortcuts = shortcuts.join(", ");
                    if block_matches(
                        &format!("{} {shortcuts}", action.label()),
                        &self.keybinding_search,
                    ) {
                        content = content.child(self.setting_row(
                            action.label(),
                            div().text_color(theme.muted).child(shortcuts),
                        ));
                    }
                }
            }
            SettingsCategory::Privacy => {
                content = content.child(
                    div()
                        .text_color(theme.muted)
                        .child("Commands are redacted locally before logging or sharing."),
                );
                for (index, pattern) in settings.privacy.redaction_patterns.iter().enumerate() {
                    content = content.child(self.text_field(
                        SharedString::from(format!("redaction-{index}")),
                        Field::Redaction(index),
                        pattern,
                        cx,
                    ));
                }
                content = content.child(
                    button("add-redaction", "Add redaction pattern", theme)
                        .px(px(8.))
                        .border_1()
                        .border_color(theme.border)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.app
                                .loaded
                                .settings
                                .privacy
                                .redaction_patterns
                                .push(String::new());
                            this.edit_field(
                                Field::Redaction(
                                    this.app.loaded.settings.privacy.redaction_patterns.len() - 1,
                                ),
                                String::new(),
                            );
                            cx.notify();
                        })),
                );
            }
        }
        for issue in &self.app.loaded.issues {
            content = content.child(
                div()
                    .text_color(theme.error)
                    .child(format!("{}: {}", issue.path, issue.message)),
            );
        }
        content
    }

    fn setting_row(&self, label: &str, control: impl IntoElement) -> gpui::Div {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(6.))
            .min_h(px(30.))
            .pb(px(6.))
            .border_b_1()
            .border_color(self.theme.border)
            .child(div().child(label.to_owned()))
            .child(control)
    }

    fn toggle(
        &self,
        label: &'static str,
        value: bool,
        update: fn(&mut Settings),
        cx: &Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        self.setting_row(
            label,
            button(
                SharedString::from(label),
                if value { "On" } else { "Off" },
                theme,
            )
            .w(px(40.))
            .border_1()
            .border_color(if value { theme.accent } else { theme.border })
            .when(value, |item| item.bg(theme.surface_high))
            .on_click(cx.listener(move |this, _, _, cx| {
                update(&mut this.app.loaded.settings);
                cx.notify();
            })),
        )
    }

    fn stepper(
        &self,
        label: &'static str,
        value: String,
        update: fn(&mut Settings, f32),
        cx: &Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        self.setting_row(
            label,
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(
                    button(SharedString::from(format!("{label}-decrease")), "−", theme)
                        .w(px(24.))
                        .border_1()
                        .border_color(theme.border)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            update(&mut this.app.loaded.settings, -1.);
                            cx.notify();
                        })),
                )
                .child(div().min_w(px(52.)).text_center().child(value))
                .child(
                    button(SharedString::from(format!("{label}-increase")), "+", theme)
                        .w(px(24.))
                        .border_1()
                        .border_color(theme.border)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            update(&mut this.app.loaded.settings, 1.);
                            cx.notify();
                        })),
                ),
        )
    }

    fn text_field(
        &self,
        id: impl Into<gpui::ElementId>,
        field: Field,
        value: &str,
        cx: &Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let value = value.to_owned();
        let active = self.field == Some(field);
        let mut control = div()
            .id(id)
            .h((self.cell.height + px(10.)).max(px(28.)))
            .w_full()
            .min_w_0()
            .px(px(6.))
            .py(px(4.))
            .rounded(px(4.))
            .border_1()
            .border_color(if active { theme.accent } else { theme.border })
            .bg(theme.background)
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener({
                    let value = value.clone();
                    move |this, event: &gpui::MouseDownEvent, window, cx| {
                        if this.field != Some(field) {
                            this.edit_field(field, value.clone());
                        } else {
                            this.field_editor.move_cursor(
                                this.input_layout.index_at(event.position),
                                event.modifiers.shift,
                            );
                        }
                        this.selecting_input = true;
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }
                }),
            );
        if active {
            let view = cx.entity();
            let editor = self.field_editor.clone();
            let style = TerminalTextStyle {
                font: self.font.clone(),
                font_size: px(self.app.loaded.settings.font.size),
                cell: self.cell,
                theme,
                drop_background_if_readable: false,
            };
            control = control.child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                            let layout = paint_input(&editor, bounds, &style, true, window, cx);
                            let focus = view.read(cx).focus.clone();
                            window.handle_input(
                                &focus,
                                ElementInputHandler::new(bounds, view.clone()),
                                cx,
                            );
                            view.update(cx, |this, _| this.input_layout = layout);
                        });
                    },
                )
                .size_full(),
            );
        } else {
            control = control.child(div().text_ellipsis().child(value));
        }
        control
    }

    pub(super) fn render_find(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let matches = self.find_matches.len();
        self.modal("find-modal", px(380.), cx)
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("Find")
                    .child(
                        button("close-find", "×", self.theme)
                            .w(px(22.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dismiss_overlay();
                                cx.notify();
                            })),
                    ),
            )
            .child(self.text_field("find-query", Field::Find, &self.find_query, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_color(self.theme.muted)
                    .text_size(px(10.))
                    .child(self.find_current.map_or_else(
                        || format!("{matches} matches"),
                        |index| format!("{} of {matches} matches", index + 1),
                    ))
                    .child(
                        button("clear-find", "Clear", self.theme)
                            .px(px(6.))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.find_query.clear();
                                this.find_current = None;
                                this.find_dirty = true;
                                this.field_editor.clear();
                                cx.notify();
                            })),
                    ),
            )
    }
}

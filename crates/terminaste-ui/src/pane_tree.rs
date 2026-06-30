use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) enum PaneTree {
    Leaf(Uuid),
    Split {
        id: Uuid,
        direction: SplitDirection,
        ratio: f32,
        first: Box<PaneTree>,
        second: Box<PaneTree>,
    },
}

impl PaneTree {
    pub fn split(&mut self, target: Uuid, new: Uuid, direction: SplitDirection) {
        match self {
            Self::Leaf(id) if *id == target => {
                *self = Self::Split {
                    id: Uuid::new_v4(),
                    direction,
                    ratio: 0.5,
                    first: Box::new(Self::Leaf(target)),
                    second: Box::new(Self::Leaf(new)),
                };
            }
            Self::Split { first, second, .. } => {
                first.split(target, new, direction);
                second.split(target, new, direction);
            }
            Self::Leaf(_) => {}
        }
    }

    pub fn layout(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        theme: &TerminasteTheme,
        panes: &mut Vec<(Uuid, egui::Rect)>,
    ) {
        match self {
            Self::Leaf(id) => panes.push((*id, rect)),
            Self::Split {
                id,
                direction,
                ratio,
                first,
                second,
            } => {
                let horizontal = *direction == SplitDirection::Right;
                let total = if horizontal {
                    rect.width()
                } else {
                    rect.height()
                };
                let usable = (total - 2.0).max(0.0);
                let minimum = 80.0_f32.min(usable / 2.0);
                let size = (usable * *ratio).clamp(minimum, (usable - minimum).max(minimum));
                let mut first_rect = rect;
                let mut second_rect = rect;
                let mut divider = rect;
                if horizontal {
                    first_rect.max.x = rect.left() + size;
                    second_rect.min.x = first_rect.right() + 2.0;
                    divider.min.x = first_rect.right();
                    divider.max.x = second_rect.left();
                } else {
                    first_rect.max.y = rect.top() + size;
                    second_rect.min.y = first_rect.bottom() + 2.0;
                    divider.min.y = first_rect.bottom();
                    divider.max.y = second_rect.top();
                }
                let response = ui
                    .interact(
                        divider.expand(4.0),
                        ui.make_persistent_id(*id),
                        Sense::drag(),
                    )
                    .on_hover_cursor(if horizontal {
                        egui::CursorIcon::ResizeHorizontal
                    } else {
                        egui::CursorIcon::ResizeVertical
                    });
                if response.dragged() && usable > 0.0 {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let coordinate = if horizontal {
                            pos.x - rect.left()
                        } else {
                            pos.y - rect.top()
                        };
                        *ratio = coordinate.clamp(minimum, usable - minimum) / usable;
                    }
                }
                ui.painter().rect_filled(divider, 0.0, theme.border);
                first.layout(ui, first_rect, theme, panes);
                second.layout(ui, second_rect, theme, panes);
            }
        }
    }
}

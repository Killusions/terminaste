use super::*;

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Add,
    Close,
    Actions,
    Settings,
}

impl Icon {
    pub fn paint(self, painter: &egui::Painter, rect: egui::Rect, color: Color32) {
        let center = rect.center();
        let point = |x, y| center + egui::vec2(x, y);
        let stroke = Stroke::new(1.3_f32, color);
        match self {
            Self::Add => {
                painter.line_segment([point(-5.0, 0.0), point(5.0, 0.0)], stroke);
                painter.line_segment([point(0.0, -5.0), point(0.0, 5.0)], stroke);
            }
            Self::Close => {
                painter.line_segment([point(-3.0, -3.0), point(3.0, 3.0)], stroke);
                painter.line_segment([point(-3.0, 3.0), point(3.0, -3.0)], stroke);
            }
            Self::Actions => {
                for y in [-4.0, 0.0, 4.0] {
                    painter.circle_filled(point(-5.0, y), 0.8, color);
                    painter.line_segment([point(-1.0, y), point(6.0, y)], stroke);
                }
            }
            Self::Settings => {
                let points = (0..16)
                    .map(|index| {
                        let angle = index as f32 * std::f32::consts::TAU / 16.0;
                        let radius = if index % 2 == 0 { 7.0 } else { 5.0 };
                        point(angle.cos() * radius, angle.sin() * radius)
                    })
                    .collect();
                painter.add(egui::Shape::closed_line(points, stroke));
                painter.circle_stroke(center, 2.2, stroke);
            }
        }
    }
}

pub(super) fn button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    icon: Icon,
    label: &str,
    theme: &TerminasteTheme,
) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click()).on_hover_text(label);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 4.0, theme.surface_high);
    }
    icon.paint(
        ui.painter(),
        rect,
        if response.hovered() {
            theme.text
        } else {
            theme.muted
        },
    );
    response
}

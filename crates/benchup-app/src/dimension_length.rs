//! Properties field that drives the geometry measured by a selected dimension.
use benchup_core::tr;
use super::*;
use benchup_core::DimensionFixedPoint;

/// The value being typed for one dimension. It follows the model while the
/// field is not being edited, so undo and other edits refresh it.
#[derive(Clone, Debug, Default)]
pub(super) struct DimensionLengthInput {
    pub(super) dimension: Option<u64>,
    pub(super) shown: String,
    pub(super) text: String,
}

pub(super) fn fixed_point_label(fixed: DimensionFixedPoint) -> &'static str {
    match fixed {
        DimensionFixedPoint::Start => tr!("dimension_length.start_point"),
        DimensionFixedPoint::End => tr!("dimension_length.end_point"),
        DimensionFixedPoint::Center => tr!("dimension_length.center"),
    }
}

/// One Properties field exists at a time; a fixed ID lets shortcuts and tests
/// address its keyboard focus.
pub(super) fn field_id() -> egui::Id {
    egui::Id::unique("dimension_length/value")
}

impl BenchUpApp {
    /// The single selected linear dimension, when it can drive geometry. The
    /// Dimensions tool selects each dimension it places, so the field is ready
    /// right away; other measuring tools and unfinished placements hide it.
    pub(super) fn selected_length_dimension(&self) -> Option<u64> {
        let placing = self.measurement.as_ref().is_some_and(|m| !m.points.is_empty());
        let measuring = self.tool.is_measuring() && (self.tool != Tool::Dimensions || placing);
        if measuring || self.selected.len() != 1 {
            return None;
        }
        let id = *self.selected.first()?;
        self.doc
            .model
            .dimensions
            .get(&id)
            .filter(|dimension| dimension.vertex.is_none())
            .map(|dimension| dimension.id)
    }

    pub(super) fn dimension_length_inspector(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected_length_dimension() else {
            self.dimension_length_input = Default::default();
            return;
        };
        let current = self.doc.model.dimensions[&id].length(&self.doc.model);
        let formatted = self.doc.display.units.format_length(current);
        let field_id = field_id();
        let focused = ui.memory(|memory| memory.has_focus(field_id));
        let input = &mut self.dimension_length_input;
        if input.dimension != Some(id) || (!focused && input.shown != formatted) {
            *input = DimensionLengthInput {
                dimension: Some(id),
                shown: formatted.clone(),
                text: formatted.clone(),
            };
        }
        let locked = self.doc.model.selection_locked(&self.selected);
        let mut submit = false;
        let response = crate::inspector_table::inspector_field(
            ui,
            "dimension_length/value",
            tr!("app.length"),
            |ui| {
                ui.add_enabled(
                    !locked,
                    egui::TextEdit::singleline(&mut self.dimension_length_input.text)
                        .id(field_id)
                        .desired_width(f32::INFINITY),
                )
            },
        );
        if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
            submit = true;
        }
        crate::inspector_table::inspector_field(
            ui,
            "dimension_length/fixed_point",
            tr!("dimension_length.keep_fixed"),
            |ui| {
                egui::ComboBox::from_id_salt("dimension_length/fixed_point")
                    .width(ui.available_width())
                    .selected_text(fixed_point_label(self.dimension_fixed_point))
                    .show_ui(ui, |ui| {
                        for fixed in DimensionFixedPoint::ALL {
                            ui.selectable_value(
                                &mut self.dimension_fixed_point,
                                fixed,
                                fixed_point_label(fixed),
                            );
                        }
                    })
                    .response
                    .on_hover_text(tr!("dimension_length.keep_fixed_hint"))
            },
        );
        let changed = self.dimension_length_input.text.trim() != formatted;
        if ui
            .add_enabled(
                !locked && changed,
                egui::Button::new(tr!("dimension_length.apply"))
                    .min_size(egui::vec2(ui.available_width(), ui.spacing().interact_size.y)),
            )
            .clicked()
        {
            submit = true;
        }
        ui.text_weak(tr!("dimension_length.resize_hint"));
        if submit && changed && !locked {
            self.apply_dimension_length(id, ui.ctx());
        }
        let mut fixed = self.doc.model.dimensions.get(&id).is_some_and(|d| d.fixed);
        if ui
            .add_enabled(!locked, egui::Checkbox::new(&mut fixed, tr!("dimension_length.fix_length")))
            .on_hover_text(tr!("dimension_length.fix_length_hint"))
            .changed()
        {
            self.set_dimension_fixed(id, fixed, ui.ctx());
        }
        ui.add_space(8.);
    }

    pub(super) fn set_dimension_fixed(&mut self, id: u64, fixed: bool, ctx: &egui::Context) {
        let before = self.doc.model.clone();
        match self.doc.model.set_dimension_fixed(id, fixed) {
            Ok(()) => self.finish_edit(
                if fixed { tr!("dimension_length.fix_dimension") } else { tr!("dimension_length.free_dimension") },
                before,
                ctx,
            ),
            Err(message) => {
                self.doc.model = before;
                self.warn(ctx, message);
            }
        }
    }

    /// Why an edit from `before` to the current model must be refused: it
    /// changes a fixed dimension other than the one being typed into.
    pub(super) fn fixed_dimension_block(&self, before: &Model) -> Option<String> {
        let id = self.doc.model.changed_fixed_dimension(before, self.driving_dimension)?;
        let length = before.dimensions[&id].length(before);
        Some(tr!(
            "dimension_length.a_fixed_dimension_holds_this_size",
            length = self.doc.display.units.format_length(length)
        ))
    }

    pub(super) fn apply_dimension_length(&mut self, id: u64, ctx: &egui::Context) {
        let text = self.dimension_length_input.text.clone();
        let length = match precision::positive_length(&text, self.doc.display.units.length) {
            Ok(length) => length,
            Err(message) => {
                self.warn(ctx, message);
                return;
            }
        };
        let before = self.doc.model.clone();
        match self
            .doc
            .model
            .set_dimension_length(id, length, self.dimension_fixed_point)
        {
            Ok(()) => {
                // The typed dimension may change its own length; others stay fixed.
                self.driving_dimension = Some(id);
                self.finish_edit(tr!("dimension_length.resize_from_dimension"), before, ctx);
                self.driving_dimension = None;
                // Show the value the model reached, in display units.
                self.dimension_length_input.dimension = None;
            }
            Err(message) => {
                self.doc.model = before;
                self.warn(ctx, message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ui::ContextExt as _;

    fn board_with_length_dimension(app: &mut BenchUpApp) -> u64 {
        let model = &mut app.doc.model;
        let points = [DVec3::ZERO, DVec3::X * 0.6, DVec3::new(0.6, 0.3, 0.), DVec3::Y * 0.3];
        for i in 0..4 {
            model.add_edge(points[i], points[(i + 1) % 4]).unwrap();
        }
        let face = *model.faces.keys().next_back().unwrap();
        model.push_pull_along(face, DVec3::Z * 0.018).unwrap();
        model.add_dimension(DVec3::ZERO, DVec3::X * 0.6, -DVec3::Y * 0.05).unwrap()
    }

    fn x_extent(model: &Model) -> f64 {
        let xs: Vec<_> = model.vertices.values().map(|v| v.position.x).collect();
        xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
            - xs.iter().cloned().fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn typed_length_resizes_the_board_as_one_undoable_edit() {
        let ctx = egui::Context::default();
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        let id = board_with_length_dimension(&mut app);
        app.selected = BTreeSet::from([id]);
        assert_eq!(app.selected_length_dimension(), Some(id));
        let _ = ctx.run_test_ui(egui::RawInput::default(), |ui| app.dimension_length_inspector(ui));
        assert_eq!(app.dimension_length_input.dimension, Some(id));
        let original = app.doc.model.clone();

        app.dimension_length_input.text = "800mm".into();
        app.apply_dimension_length(id, &ctx);

        assert!((x_extent(&app.doc.model) - 0.8).abs() < 1e-7);
        assert!(!app.doc.history.is_empty());
        assert_eq!(app.selected, BTreeSet::from([id]));
        // The field shows the reached value again on the next frame.
        let _ = ctx.run_test_ui(egui::RawInput::default(), |ui| app.dimension_length_inspector(ui));
        assert_eq!(
            app.dimension_length_input.text,
            app.doc.display.units.format_length(0.8)
        );
        app.undo(&ctx, false);
        assert!((x_extent(&app.doc.model) - 0.6).abs() < 1e-7);
        assert!(app.doc.model.same_scene_geometry(&original));
    }

    #[test]
    fn invalid_lengths_report_errors_without_editing() {
        let ctx = egui::Context::default();
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        let id = board_with_length_dimension(&mut app);
        app.selected = BTreeSet::from([id]);
        let before = app.doc.model.clone();
        for text in ["", "abc", "-5mm", "0"] {
            app.dimension_length_input.text = text.into();
            app.apply_dimension_length(id, &ctx);
            assert_eq!(app.doc.model, before, "{text}");
        }
        assert!(app.doc.history.is_empty());
    }

    #[test]
    fn measuring_tools_and_angle_dimensions_do_not_offer_the_field() {
        let mut app = BenchUpApp::with_document(Document::default(), None, true);
        let id = board_with_length_dimension(&mut app);
        let angle = app
            .doc
            .model
            .add_angular_dimension(DVec3::ZERO, DVec3::X * 0.6, DVec3::Y * 0.3)
            .unwrap();
        app.selected = BTreeSet::from([angle]);
        assert_eq!(app.selected_length_dimension(), None);
        app.selected = BTreeSet::from([id]);
        // The Dimensions tool keeps the field available between placements…
        app.set_tool(Tool::Dimensions);
        app.selected = BTreeSet::from([id]);
        assert_eq!(app.selected_length_dimension(), Some(id));
        // …but not while a new dimension is being placed.
        app.measurement = Some(measurement_tools::Measurement {
            points: vec![DVec3::ZERO],
            ..Default::default()
        });
        assert_eq!(app.selected_length_dimension(), None);
        app.set_tool(Tool::TapeMeasure);
        app.selected = BTreeSet::from([id]);
        assert_eq!(app.selected_length_dimension(), None);
    }
}

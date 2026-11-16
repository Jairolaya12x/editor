use crate::{DVec3, DimensionFixedPoint, Model};
use std::collections::BTreeSet;

const LENGTH: f64 = 0.6;
const WIDTH: f64 = 0.3;
const THICKNESS: f64 = 0.018;

/// A 600 × 300 × 18 mm board as loose geometry starting at `origin`.
fn board(model: &mut Model, origin: DVec3) {
    let points = [
        origin,
        origin + DVec3::X * LENGTH,
        origin + DVec3::new(LENGTH, WIDTH, 0.),
        origin + DVec3::Y * WIDTH,
    ];
    for i in 0..4 {
        model.add_edge(points[i], points[(i + 1) % 4]).unwrap();
    }
    let face = *model.faces.keys().next_back().unwrap();
    model.push_pull_along(face, DVec3::Z * THICKNESS).unwrap();
}

fn x_range(model: &Model) -> (f64, f64) {
    model.vertices.values().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
        (lo.min(v.position.x), hi.max(v.position.x))
    })
}

fn extent(model: &Model, axis: fn(DVec3) -> f64) -> f64 {
    let values: Vec<_> = model.vertices.values().map(|v| axis(v.position)).collect();
    values.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        - values.iter().cloned().fold(f64::INFINITY, f64::min)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-7
}

#[test]
fn length_dimension_stretches_a_board_from_each_fixed_point() {
    for (fixed, expected) in [
        (DimensionFixedPoint::Start, (0., 0.8)),
        (DimensionFixedPoint::End, (-0.2, 0.6)),
        (DimensionFixedPoint::Center, (-0.1, 0.7)),
    ] {
        let mut model = Model::default();
        board(&mut model, DVec3::ZERO);
        let dimension = model
            .add_dimension(DVec3::ZERO, DVec3::X * LENGTH, -DVec3::Y * 0.05)
            .unwrap();
        let topology = (model.vertices.len(), model.edges.len(), model.faces.len());

        model.set_dimension_length(dimension, 0.8, fixed).unwrap();

        let (lo, hi) = x_range(&model);
        assert!(close(lo, expected.0) && close(hi, expected.1), "{fixed:?}: {lo} {hi}");
        assert!(close(model.dimensions[&dimension].length(&model), 0.8));
        // Stretching keeps the thickness and width and adds no topology.
        assert!(close(extent(&model, |p| p.z), THICKNESS));
        assert!(close(extent(&model, |p| p.y), WIDTH));
        assert_eq!(topology, (model.vertices.len(), model.edges.len(), model.faces.len()));
        model.validate().unwrap();
    }
}

#[test]
fn shortening_a_board_keeps_it_valid_until_it_would_fold() {
    let mut model = Model::default();
    board(&mut model, DVec3::ZERO);
    let dimension = model
        .add_dimension(DVec3::ZERO, DVec3::X * LENGTH, -DVec3::Y * 0.05)
        .unwrap();
    model
        .set_dimension_length(dimension, 0.1, DimensionFixedPoint::Start)
        .unwrap();
    let (lo, hi) = x_range(&model);
    assert!(close(lo, 0.) && close(hi, 0.1));
    model.validate().unwrap();

    // A dimension spanning part of the board cannot be shortened past the
    // vertices that stay behind.
    let mut model = Model::default();
    board(&mut model, DVec3::ZERO);
    let partial = model
        .add_dimension(DVec3::X * 0.1, DVec3::X * LENGTH, -DVec3::Y * 0.05)
        .unwrap();
    let before = model.clone();
    assert!(model
        .set_dimension_length(partial, 0.01, DimensionFixedPoint::Start)
        .is_err());
    assert_eq!(model, before);
}

#[test]
fn width_and_thickness_dimensions_drive_their_own_axes() {
    let mut model = Model::default();
    board(&mut model, DVec3::ZERO);
    let width = model
        .add_dimension(DVec3::ZERO, DVec3::Y * WIDTH, -DVec3::X * 0.05)
        .unwrap();
    let thickness = model
        .add_dimension(DVec3::ZERO, DVec3::Z * THICKNESS, -DVec3::X * 0.05)
        .unwrap();
    model.set_dimension_length(width, 0.4, DimensionFixedPoint::Start).unwrap();
    model.set_dimension_length(thickness, 0.019, DimensionFixedPoint::Start).unwrap();
    assert!(close(extent(&model, |p| p.x), LENGTH));
    assert!(close(extent(&model, |p| p.y), 0.4));
    assert!(close(extent(&model, |p| p.z), 0.019));
    model.validate().unwrap();
}

#[test]
fn stretching_a_component_instance_updates_its_linked_peers() {
    let mut model = Model::default();
    board(&mut model, DVec3::ZERO);
    let component = model
        .make_object(&model.selectable_entities(), true, "Board")
        .unwrap();
    let clipboard = model.copy_entities(&BTreeSet::from([component]));
    let peer = *model.paste_entities(&clipboard, DVec3::Y * 2.).first().unwrap();
    let dimension = model
        .add_dimension(DVec3::ZERO, DVec3::X * LENGTH, -DVec3::Y * 0.05)
        .unwrap();

    model
        .set_dimension_length(dimension, 0.75, DimensionFixedPoint::Start)
        .unwrap();

    for id in [component, peer] {
        let contents = model.object_contents(id).unwrap();
        assert!(close(extent(&contents, |p| p.x), 0.75), "object {id}");
        assert!(close(extent(&contents, |p| p.z), THICKNESS));
    }
    assert!(close(model.dimensions[&dimension].length(&model), 0.75));
    assert_eq!(model.edit_context, None);
    model.validate().unwrap();
}

#[test]
fn dimension_between_two_groups_moves_the_free_group() {
    let mut model = Model::default();
    board(&mut model, DVec3::ZERO);
    let first = model
        .make_object(&model.selectable_entities(), false, "Left")
        .unwrap();
    board(&mut model, DVec3::X * 1.);
    let loose = model.selectable_entities();
    let loose: BTreeSet<_> = loose.into_iter().filter(|id| *id != first).collect();
    let second = model.make_object(&loose, false, "Right").unwrap();
    // From the left board's right end to the right board's left end.
    let gap = model
        .add_dimension(DVec3::X * LENGTH, DVec3::X * 1., -DVec3::Y * 0.05)
        .unwrap();
    let left_before = model.object_contents(first).unwrap();

    model
        .set_dimension_length(gap, 0.5, DimensionFixedPoint::Start)
        .unwrap();

    assert_eq!(model.object_contents(first).unwrap().vertices, left_before.vertices);
    let right = model.object_contents(second).unwrap();
    assert!(close(x_range(&right).0, 1.1));
    assert!(close(extent(&right, |p| p.x), LENGTH));
    assert!(close(model.dimensions[&gap].length(&model), 0.5));
    model.validate().unwrap();
}

#[test]
fn invalid_requests_leave_the_model_unchanged() {
    let mut model = Model::default();
    board(&mut model, DVec3::ZERO);
    let dimension = model
        .add_dimension(DVec3::ZERO, DVec3::X * LENGTH, -DVec3::Y * 0.05)
        .unwrap();
    let floating = model
        .add_dimension(DVec3::new(5., 5., 5.), DVec3::new(6., 5., 5.), DVec3::Y)
        .unwrap();
    let angle = model
        .add_angular_dimension(DVec3::ZERO, DVec3::X * LENGTH, DVec3::Y * WIDTH)
        .unwrap();
    let before = model.clone();
    for (id, length) in [
        (dimension, 0.),
        (dimension, -1.),
        (dimension, f64::NAN),
        (floating, 2.),
        (angle, 1.),
        (u64::MAX, 1.),
    ] {
        assert!(model
            .set_dimension_length(id, length, DimensionFixedPoint::Start)
            .is_err());
        assert_eq!(model, before);
    }
    // Unchanged values are accepted without editing the model.
    model
        .set_dimension_length(dimension, LENGTH, DimensionFixedPoint::Center)
        .unwrap();
    assert_eq!(model, before);
}

#[test]
fn locked_geometry_cannot_be_stretched() {
    let mut model = Model::default();
    board(&mut model, DVec3::ZERO);
    let group = model
        .make_object(&model.selectable_entities(), false, "Board")
        .unwrap();
    model.locked.insert(group);
    let dimension = model
        .add_dimension(DVec3::ZERO, DVec3::X * LENGTH, -DVec3::Y * 0.05)
        .unwrap();
    let before = model.clone();
    assert!(model
        .set_dimension_length(dimension, 0.8, DimensionFixedPoint::Start)
        .is_err());
    assert_eq!(model, before);
}

#[test]
fn stretched_geometry_stays_editable_after_the_dimension_is_erased() {
    // Loose board, then a board inside a group (edited in context).
    for grouped in [false, true] {
        let mut model = Model::default();
        board(&mut model, DVec3::ZERO);
        let group = grouped.then(|| {
            model.make_object(&model.selectable_entities(), false, "Board").unwrap()
        });
        let dimension = model
            .add_dimension(DVec3::ZERO, DVec3::X * LENGTH, -DVec3::Y * 0.05)
            .unwrap();
        model
            .set_dimension_length(dimension, 0.8, DimensionFixedPoint::Start)
            .unwrap();
        model.erase(&BTreeSet::from([dimension]));
        assert!(model.dimensions.is_empty());
        if let Some(group) = group {
            assert!(model.enter_object(group));
        }
        let top = model
            .faces
            .values()
            .find(|face| {
                face.vertices
                    .iter()
                    .all(|id| (model.vertices[id].position.z - THICKNESS).abs() < 1e-7)
            })
            .unwrap()
            .id;
        let result = model.push_pull(top, 0.01);
        assert!(result.is_ok(), "grouped={grouped}: {result:?}");
        assert!(close(extent(&model, |p| p.z), THICKNESS + 0.01));
        model.validate().unwrap();
    }
}

#[test]
fn every_face_of_a_stretched_cube_can_be_pushed_and_pulled() {
    use crate::Axis;
    for axis in [None, Some(Axis::X)] {
        for erase in [false, true] {
            for side in 0..6 {
                let mut model = Model::default();
                let points = [DVec3::ZERO, DVec3::X, DVec3::new(1., 1., 0.), DVec3::Y];
                for i in 0..4 {
                    model.add_edge(points[i], points[(i + 1) % 4]).unwrap();
                }
                let face = *model.faces.keys().next_back().unwrap();
                model.push_pull_along(face, DVec3::Z).unwrap();
                let dimension = model
                    .add_dimension_with_axis(DVec3::ZERO, DVec3::X, -DVec3::Y, axis)
                    .unwrap();
                model
                    .set_dimension_length(dimension, 2., DimensionFixedPoint::Start)
                    .unwrap();
                if erase {
                    model.erase(&BTreeSet::from([dimension]));
                }
                let mut faces: Vec<_> = model.faces.keys().copied().collect();
                faces.sort();
                let face = faces[side];
                let before = model.clone();
                let result = model.push_pull(face, 0.25);
                assert!(
                    result.is_ok(),
                    "axis={axis:?} erase={erase} face#{side}: {result:?}"
                );
                model.validate().unwrap();
                let _ = before;
            }
        }
    }
}

#[test]
fn fixed_dimensions_report_changes_and_survive_serialization() {
    let mut model = Model::default();
    board(&mut model, DVec3::ZERO);
    let length = model
        .add_dimension(DVec3::ZERO, DVec3::X * LENGTH, -DVec3::Y * 0.05)
        .unwrap();
    assert!(!model.dimensions[&length].fixed, "dimensions start free");
    model.set_dimension_fixed(length, true).unwrap();
    // Older documents without the field load as free dimensions.
    let json = serde_json::to_string(&model.dimensions[&length]).unwrap();
    assert!(json.contains("\"fixed\":true"));
    let legacy: crate::Dimension = serde_json::from_str(&json.replace(",\"fixed\":true", "")).unwrap();
    assert!(!legacy.fixed);

    let before = model.clone();
    let mut pulled = model.clone();
    let end = *pulled
        .faces
        .iter()
        .find(|(_, f)| f.vertices.iter().all(|id| (pulled.vertices[id].position.x - LENGTH).abs() < 1e-7))
        .unwrap()
        .0;
    pulled.push_pull(end, 0.1).unwrap();
    assert_eq!(pulled.changed_fixed_dimension(&before, None), Some(length));
    assert_eq!(pulled.changed_fixed_dimension(&before, Some(length)), None);

    let mut top = model.clone();
    let face = *top
        .faces
        .iter()
        .find(|(_, f)| f.vertices.iter().all(|id| (top.vertices[id].position.z - THICKNESS).abs() < 1e-7))
        .unwrap()
        .0;
    top.push_pull(face, 0.01).unwrap();
    assert_eq!(top.changed_fixed_dimension(&before, None), None, "other sizes stay editable");

    model.set_dimension_fixed(length, false).unwrap();
    assert_eq!(pulled.changed_fixed_dimension(&model, None), None);
}

#[test]
fn driving_one_dimension_respects_other_fixed_dimensions() {
    let mut model = Model::default();
    board(&mut model, DVec3::ZERO);
    board(&mut model, DVec3::X);
    // A fixed gap between the boards, and the right board's length.
    let gap = model
        .add_dimension(DVec3::X * LENGTH, DVec3::X, -DVec3::Y * 0.1)
        .unwrap();
    let length = model
        .add_dimension(DVec3::X, DVec3::X * (1. + LENGTH), -DVec3::Y * 0.05)
        .unwrap();
    model.set_dimension_fixed(gap, true).unwrap();
    model.set_dimension_fixed(length, true).unwrap();
    // Moving the far end leaves the gap unchanged…
    model.set_dimension_length(length, 0.8, DimensionFixedPoint::Start).unwrap();
    assert!(close(model.dimensions[&gap].length(&model), 0.4));
    // …while moving the near end would close the gap, so it is refused atomically.
    let before = model.clone();
    assert!(model
        .set_dimension_length(length, 0.9, DimensionFixedPoint::End)
        .is_err());
    assert_eq!(model, before);
}

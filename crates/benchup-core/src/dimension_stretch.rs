//! Driving dimensions: typing a new value into a linear dimension resizes or
//! moves the geometry it measures.
//!
//! When both ends are attached to the same connected piece, the piece is
//! stretched: vertices beyond the plane halfway between the two ends move
//! along the dimension, so a board gets longer without changing its thickness.
//! When the ends belong to different pieces, the moving piece is translated.
use crate::{DVec3, Model, document::EPS};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The point of a linear dimension that stays in place when its value changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionFixedPoint {
    /// The first endpoint chosen when the dimension was created.
    #[default]
    Start,
    /// The second endpoint chosen when the dimension was created.
    End,
    /// Both ends move by half of the change.
    Center,
}

impl DimensionFixedPoint {
    pub const ALL: [Self; 3] = [Self::Start, Self::End, Self::Center];
}

/// Geometry that one end of the dimension is attached to.
struct Side {
    /// The object whose direct geometry contains the anchor, or `None` for
    /// loose geometry at the model root.
    owner: Option<u64>,
    /// Vertices connected to the anchor through edges of the same owner.
    piece: BTreeSet<u64>,
}

impl Model {
    /// Set a linear dimension to `length` by stretching or moving the geometry
    /// it is attached to. The change is atomic: on error the model is unchanged.
    pub fn set_dimension_length(
        &mut self,
        id: u64,
        length: f64,
        fixed: DimensionFixedPoint,
    ) -> Result<(), &'static str> {
        let dimension = self
            .dimensions
            .get(&id)
            .ok_or(tr!("core.dimension_stretch.the_dimension_no_longer_exists"))?
            .clone();
        if dimension.vertex.is_some() {
            return Err(tr!("core.dimension_stretch.angle_dimensions_cannot_resize_geometry"));
        }
        if !length.is_finite() || length < EPS || length > 1e7 {
            return Err(tr!("core.dimension_stretch.enter_a_positive_length"));
        }
        let current = dimension.length(self);
        let change = length - current;
        if change.abs() < EPS {
            return Ok(());
        }
        let (a, b) = dimension.points(self);
        let direction = dimension.line_direction(self);
        // Unit vector pointing from the start toward the end of the dimension.
        let toward_end = if (b - a).dot(direction) < 0. { -direction } else { direction };
        let (start_shift, end_shift) = match fixed {
            DimensionFixedPoint::Start => (0., change),
            DimensionFixedPoint::End => (-change, 0.),
            DimensionFixedPoint::Center => (-change * 0.5, change * 0.5),
        };

        let parents = self.parent_map();
        let side = |anchor: &crate::DimensionAnchor| -> Option<Side> {
            let vertex = anchor.weights.first()?.0;
            let owner = self.vertex_owner(vertex, &parents)?;
            let mut piece = self.connected_vertices(
                anchor.weights.iter().map(|(id, _)| *id),
                owner,
                &parents,
            );
            piece.extend(anchor.weights.iter().map(|(id, _)| *id));
            Some(Side { owner: owner.copied(), piece })
        };
        let start = side(&dimension.start);
        let end = side(&dimension.end);
        let detached = tr!("core.dimension_stretch.attach_the_moving_end_to_geometry");
        if start_shift != 0. && start.is_none() || end_shift != 0. && end.is_none() {
            return Err(detached);
        }

        let mut next = self.clone();
        match (&start, &end) {
            (Some(s), Some(e)) if s.owner == e.owner && !s.piece.is_disjoint(&e.piece) => {
                // Both ends on one piece: stretch it about the midpoint.
                let middle = (a + b) * 0.5;
                let moves = stretch_moves(
                    self,
                    &e.piece,
                    middle,
                    toward_end,
                    start_shift,
                    end_shift,
                )?;
                next.in_scope(e.owner, |model| model.shift_vertices(&moves))?;
            }
            _ => {
                for (side, shift) in [(&start, start_shift), (&end, end_shift)] {
                    let Some(side) = side.as_ref().filter(|_| shift != 0.) else {
                        continue;
                    };
                    next.move_piece(side, &start, &end, toward_end * shift)?;
                }
            }
        }
        let reached = next
            .dimensions
            .get(&id)
            .map(|d| d.length(&next))
            .is_some_and(|value| (value - length).abs() <= 1e-6_f64.max(length * 1e-9));
        if !reached {
            return Err(tr!("core.dimension_stretch.this_dimension_cannot_resize_its_geometry"));
        }
        if next.changed_fixed_dimension(self, Some(id)).is_some() {
            return Err(tr!("core.dimension_stretch.another_fixed_dimension_holds_this_size"));
        }
        next.edit_context = self.edit_context;
        *self = next;
        Ok(())
    }

    /// The first fixed linear dimension, other than `except`, whose length
    /// differs from the one it had in `before`. A dimension with an end that
    /// is not attached to geometry in either model holds nothing.
    pub fn changed_fixed_dimension(&self, before: &Model, except: Option<u64>) -> Option<u64> {
        let holds = |d: &crate::Dimension| {
            d.fixed && d.vertex.is_none() && !d.start.weights.is_empty() && !d.end.weights.is_empty()
        };
        self.dimensions
            .values()
            .filter(|d| Some(d.id) != except && holds(d))
            .find(|d| {
                before.dimensions.get(&d.id).is_some_and(|old| {
                    holds(old) && (old.length(before) - d.length(self)).abs() > 1e-6
                })
            })
            .map(|d| d.id)
    }

    /// Fix or free a linear dimension's length.
    pub fn set_dimension_fixed(&mut self, id: u64, fixed: bool) -> Result<(), &'static str> {
        if self.edit_context.is_some() && self.parent(id).is_some() {
            return self.edit_in_context(|model| model.set_dimension_fixed(id, fixed));
        }
        let dimension = self
            .dimensions
            .get_mut(&id)
            .ok_or(tr!("core.dimension_stretch.the_dimension_no_longer_exists"))?;
        if dimension.vertex.is_some() {
            return Err(tr!("core.dimension_stretch.angle_dimensions_cannot_resize_geometry"));
        }
        dimension.fixed = fixed;
        Ok(())
    }

    /// Translate the piece on one side of a dimension whose ends belong to
    /// different pieces. A whole object moves when it is the anchor's owner and
    /// does not also contain the other end.
    fn move_piece(
        &mut self,
        side: &Side,
        start: &Option<Side>,
        end: &Option<Side>,
        delta: DVec3,
    ) -> Result<(), &'static str> {
        let other_owners: Vec<_> = [start, end]
            .into_iter()
            .flatten()
            .filter(|other| !std::ptr::eq(*other, side))
            .map(|other| other.owner)
            .collect();
        if let Some(object) = side.owner
            && other_owners
                .iter()
                .all(|owner| owner.is_none_or(|owner| !self.is_ancestor_or_self(object, owner)))
        {
            let scope = self.parent(object);
            let selection = BTreeSet::from([object]);
            return self.in_scope(scope, |model| model.move_selection(&selection, delta));
        }
        let moves = side.piece.iter().map(|id| (*id, delta)).collect();
        self.in_scope(side.owner, |model| model.shift_vertices(&moves))
    }

    /// Run `operation` with `scope` as the editing context, so edits inside a
    /// component instance also update its linked peers.
    fn in_scope<T>(
        &mut self,
        scope: Option<u64>,
        operation: impl FnOnce(&mut Model) -> Result<T, &'static str>,
    ) -> Result<T, &'static str> {
        let saved = self.edit_context;
        let mut next = self.clone();
        next.edit_context = scope;
        let result = match scope {
            Some(_) => next.edit_in_context(operation),
            None => operation(&mut next),
        }?;
        next.edit_context = saved;
        *self = next;
        Ok(result)
    }

    /// Displace individual vertices in the current scope, then validate.
    fn shift_vertices(&mut self, moves: &BTreeMap<u64, DVec3>) -> Result<(), &'static str> {
        if moves.is_empty() {
            return Ok(());
        }
        let touches_locked = self.edges.values().any(|edge| {
            (moves.contains_key(&edge.a) || moves.contains_key(&edge.b)) && self.is_locked(edge.id)
        });
        if touches_locked {
            return Err(tr!("core.modeling.unlock_the_connected_geometry_before_editing_it"));
        }
        let mut next = self.clone();
        for (id, delta) in moves {
            let vertex = next
                .vertices
                .get_mut(id)
                .ok_or(tr!("core.dimension_stretch.this_dimension_cannot_resize_its_geometry"))?;
            vertex.position += *delta;
        }
        next.sync_dimensions();
        next.validate()
            .map_err(|_| tr!("core.modeling.this_change_would_collapse_an_edge_or_bend"))?;
        *self = next;
        Ok(())
    }

    fn parent_map(&self) -> BTreeMap<u64, u64> {
        self.objects
            .values()
            .flat_map(|object| object.entities.iter().map(|id| (*id, object.id)))
            .collect()
    }

    /// The owner of a vertex's edges: `Some(None)` for root geometry.
    fn vertex_owner<'a>(
        &self,
        vertex: u64,
        parents: &'a BTreeMap<u64, u64>,
    ) -> Option<Option<&'a u64>> {
        self.edges
            .values()
            .find(|edge| edge.a == vertex || edge.b == vertex)
            .map(|edge| parents.get(&edge.id))
    }

    /// Vertices reachable from `seeds` through edges directly owned by `owner`.
    fn connected_vertices(
        &self,
        seeds: impl IntoIterator<Item = u64>,
        owner: Option<&u64>,
        parents: &BTreeMap<u64, u64>,
    ) -> BTreeSet<u64> {
        let mut neighbors: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        for edge in self.edges.values().filter(|edge| parents.get(&edge.id) == owner) {
            neighbors.entry(edge.a).or_default().push(edge.b);
            neighbors.entry(edge.b).or_default().push(edge.a);
        }
        let mut found = BTreeSet::new();
        let mut pending: Vec<_> = seeds.into_iter().collect();
        while let Some(id) = pending.pop() {
            if found.insert(id) {
                pending.extend(neighbors.get(&id).into_iter().flatten().copied());
            }
        }
        found
    }

    fn is_ancestor_or_self(&self, ancestor: u64, mut id: u64) -> bool {
        loop {
            if id == ancestor {
                return true;
            }
            match self.parent(id) {
                Some(parent) => id = parent,
                None => return false,
            }
        }
    }
}

/// Vertex displacements that stretch one piece about a plane through `middle`.
/// Rejects shortening that would push vertices past ones that stay behind.
fn stretch_moves(
    model: &Model,
    piece: &BTreeSet<u64>,
    middle: DVec3,
    toward_end: DVec3,
    start_shift: f64,
    end_shift: f64,
) -> Result<BTreeMap<u64, DVec3>, &'static str> {
    let mut moves = BTreeMap::new();
    // Furthest extents after the change, per group: (start side, middle, end side).
    let mut start_max = f64::NEG_INFINITY;
    let mut end_min = f64::INFINITY;
    let mut middle_range = (f64::INFINITY, f64::NEG_INFINITY);
    for id in piece {
        let t = (model.vertices[id].position - middle).dot(toward_end);
        if t > EPS {
            end_min = end_min.min(t + end_shift);
            if end_shift != 0. {
                moves.insert(*id, toward_end * end_shift);
            }
        } else if t < -EPS {
            start_max = start_max.max(t + start_shift);
            if start_shift != 0. {
                moves.insert(*id, toward_end * start_shift);
            }
        } else {
            middle_range = (middle_range.0.min(t), middle_range.1.max(t));
        }
    }
    let lowest_after_start = middle_range.0.min(end_min);
    let highest_before_end = middle_range.1.max(start_max);
    if end_min <= highest_before_end + EPS || start_max >= lowest_after_start - EPS {
        return Err(tr!("core.dimension_stretch.that_length_would_fold_the_geometry"));
    }
    Ok(moves)
}

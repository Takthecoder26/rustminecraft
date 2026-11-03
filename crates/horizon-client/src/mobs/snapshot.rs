//! Observable entity reads made by a region transaction. An unchanged query
//! result includes negative space: an earlier entity entering an empty queried
//! cell invalidates the prediction. No distance/velocity halo is assumed.
use super::{
    Seen,
    seen_grid::{Cell, Columns, SeenGrid, SnapshotGrid, corner},
};
use crate::fast_hash::{FxHashMap, FxHashSet};
use horizon_core::geometry::Aabb;
use std::{cell::RefCell, ops::Index};

#[derive(Default, Debug)]
pub struct ReadSet {
    all: bool,
    slots: FxHashSet<usize>,
    queries: Vec<(Aabb, Option<Columns>)>,
}
impl ReadSet {
    pub fn query(&mut self, area: Aabb, columns: Option<Columns>) {
        self.queries.push((area, columns));
    }
}

/// Mutations already published in the original global entity order. Owner
/// identity is frozen for this tick, even when an entity migrates spatially.
pub struct SnapshotChanges {
    owners: Vec<usize>,
    slots: Vec<bool>,
    cells: FxHashMap<Cell, Vec<usize>>,
    owners_ticked: FxHashSet<usize>,
}
impl SnapshotChanges {
    pub fn new(owners: Vec<usize>) -> Self {
        Self {
            slots: vec![false; owners.len()],
            owners,
            cells: Default::default(),
            owners_ticked: Default::default(),
        }
    }
    pub fn touch(&mut self, slot: usize, before: Aabb, after: Aabb) {
        let owner = self.owners[slot];
        self.slots[slot] = true;
        self.owners_ticked.insert(owner);
        for cell in [corner(before), corner(after)] {
            let owners = self.cells.entry(cell).or_default();
            if !owners.contains(&owner) {
                owners.push(owner);
            }
        }
    }
    pub fn valid(&self, reads: &ReadSet, owner: usize, grid: &SeenGrid) -> bool {
        if reads.all && self.owners_ticked.iter().any(|&other| other != owner) {
            return false;
        }
        if reads
            .slots
            .iter()
            .any(|&slot| self.slots[slot] && self.owners[slot] != owner)
        {
            return false;
        }
        for &(area, expected_columns) in &reads.queries {
            // Growing boxes and retained/new grid cells can change both the
            // candidate range and full-scan fallback. Compare the real shape.
            if grid.columns(area) != expected_columns {
                return false;
            }
            if let Some(columns) = expected_columns {
                for cell in columns.cells() {
                    if self
                        .cells
                        .get(&cell)
                        .is_some_and(|owners| owners.iter().any(|&other| other != owner))
                    {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Only one region's changed entries, over the tick's borrowed base snapshot.
#[derive(Default)]
pub struct SnapshotOverlay {
    entries: FxHashMap<usize, Seen>,
}
impl SnapshotOverlay {
    pub fn update(&mut self, slot: usize, seen: Seen) {
        self.entries.insert(slot, seen);
    }
}

/// Prevents unobserved raw-slice scans in mob implementations. Point lookups
/// record their slot; iteration records a conservative complete-snapshot read.
/// Spatial queries use get_untracked only after recording their queried cells.
#[derive(Clone, Copy)]
pub struct SeenView<'a> {
    base: &'a [Seen],
    overlay: Option<&'a SnapshotOverlay>,
    reads: Option<&'a RefCell<ReadSet>>,
}
impl<'a> SeenView<'a> {
    pub fn observed(
        base: &'a [Seen],
        overlay: &'a SnapshotOverlay,
        reads: &'a RefCell<ReadSet>,
    ) -> Self {
        Self {
            base,
            overlay: Some(overlay),
            reads: Some(reads),
        }
    }
    pub fn len(self) -> usize {
        self.base.len()
    }
    pub fn is_empty(self) -> bool {
        self.base.is_empty()
    }
    pub fn get(self, slot: usize) -> Option<&'a Seen> {
        let value = self.get_untracked(slot);
        if value.is_some()
            && let Some(reads) = self.reads
        {
            reads.borrow_mut().slots.insert(slot);
        }
        value
    }
    pub(super) fn get_untracked(self, slot: usize) -> Option<&'a Seen> {
        self.overlay
            .and_then(|o| o.entries.get(&slot))
            .or_else(|| self.base.get(slot))
    }
    pub(super) fn record_query(self, area: Aabb, grid: &dyn SnapshotGrid) {
        if let Some(reads) = self.reads {
            reads.borrow_mut().query(area, grid.columns(area));
        }
    }
    pub fn iter(self) -> SeenIter<'a> {
        if let Some(reads) = self.reads {
            reads.borrow_mut().all = true;
        }
        SeenIter {
            view: self,
            slot: 0,
        }
    }
}
impl<'a> From<&'a [Seen]> for SeenView<'a> {
    fn from(base: &'a [Seen]) -> Self {
        Self {
            base,
            overlay: None,
            reads: None,
        }
    }
}
impl<'a> From<&'a Vec<Seen>> for SeenView<'a> {
    fn from(base: &'a Vec<Seen>) -> Self {
        base.as_slice().into()
    }
}
impl<'a, const N: usize> From<&'a [Seen; N]> for SeenView<'a> {
    fn from(base: &'a [Seen; N]) -> Self {
        base.as_slice().into()
    }
}
impl Index<usize> for SeenView<'_> {
    type Output = Seen;
    fn index(&self, slot: usize) -> &Seen {
        self.get(slot).expect("snapshot slot")
    }
}
pub struct SeenIter<'a> {
    view: SeenView<'a>,
    slot: usize,
}
impl<'a> Iterator for SeenIter<'a> {
    type Item = &'a Seen;
    fn next(&mut self) -> Option<Self::Item> {
        let value = self.view.get_untracked(self.slot);
        self.slot += usize::from(value.is_some());
        value
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.view.len() - self.slot;
        (left, Some(left))
    }
}
impl ExactSizeIterator for SeenIter<'_> {}
impl<'a> IntoIterator for SeenView<'a> {
    type Item = &'a Seen;
    type IntoIter = SeenIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<'a> IntoIterator for &SeenView<'a> {
    type Item = &'a Seen;
    type IntoIter = SeenIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::Vec3;
    fn bounds(x: f64, width: f64) -> Aabb {
        Aabb::new(Vec3::new(x, 0.0, 0.0), Vec3::new(x + width, 1.8, width))
    }
    fn grid() -> SeenGrid {
        SeenGrid::new((0..128).map(|i| bounds(f64::from(i) * 8.0, 0.6)))
    }

    #[test]
    fn entering_a_previously_empty_query_cell_invalidates_a_prediction() {
        let mut grid = grid();
        let mut changes = SnapshotChanges::new((0..128).collect());
        let area = bounds(32.0, 1.0);
        let mut reads = ReadSet::default();
        reads.query(area, grid.columns(area));
        assert!(changes.valid(&reads, 0, &grid));
        // An unrelated distant tick has no effect on this query.
        changes.touch(100, bounds(800.0, 0.6), bounds(800.1, 0.6));
        grid.update(100, bounds(800.1, 0.6));
        assert!(changes.valid(&reads, 0, &grid));
        // This is an actual destination, not a maximum-speed halo.
        changes.touch(101, bounds(808.0, 0.6), bounds(32.2, 0.6));
        grid.update(101, bounds(32.2, 0.6));
        assert!(!changes.valid(&reads, 0, &grid));
    }

    #[test]
    fn point_and_unbounded_reads_detect_distant_changes() {
        let grid = grid();
        let mut changes = SnapshotChanges::new((0..128).collect());
        let mut point = ReadSet::default();
        point.slots.insert(100);
        let all = ReadSet {
            all: true,
            ..Default::default()
        };
        changes.touch(0, bounds(0.0, 0.6), bounds(0.1, 0.6));
        // Own region prefixes are validated by the ordered transaction commit.
        assert!(changes.valid(&point, 0, &grid));
        assert!(changes.valid(&all, 0, &grid));
        changes.touch(100, bounds(800.0, 0.6), bounds(800.1, 0.6));
        assert!(!changes.valid(&point, 0, &grid));
        assert!(!changes.valid(&all, 0, &grid));
    }

    #[test]
    fn a_growing_foreign_box_changes_the_real_query_shape() {
        let mut grid = grid();
        let changes = SnapshotChanges::new((0..128).collect());
        let area = bounds(32.0, 1.0);
        let mut reads = ReadSet::default();
        reads.query(area, grid.columns(area));
        // Even if the changed body's old corner is distant, its larger width
        // changes the grid's conservative candidate range globally.
        grid.update(100, bounds(800.0, 20.0));
        assert!(!changes.valid(&reads, 0, &grid));
    }
}

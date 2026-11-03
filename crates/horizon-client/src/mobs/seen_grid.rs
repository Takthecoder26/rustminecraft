//! Ordered snapshot box queries and sparse per-region speculative overlays.
//! An overlay copies only cells changed by its own entities. It reproduces the
//! serial grid's candidate order, growing reach and retained empty cells.
use crate::fast_hash::FxHashMap;
use horizon_core::geometry::{Aabb, Axis};

const CELL: f64 = 4.0;
pub type Cell = (i32, i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Columns {
    pub lo: Cell,
    pub hi: Cell,
}
impl Columns {
    pub fn cells(self) -> impl Iterator<Item = Cell> {
        (self.lo.0..=self.hi.0).flat_map(move |x| (self.lo.1..=self.hi.1).map(move |z| (x, z)))
    }
}

/// The same query interface for a committed snapshot and a speculative overlay.
pub trait SnapshotGrid: Sync {
    /// None means the serial grid falls back to the complete snapshot.
    fn columns(&self, area: Aabb) -> Option<Columns>;
    fn candidates(&self, area: Aabb, out: &mut Vec<u32>) -> bool;
}

#[derive(Default)]
pub struct SeenGrid {
    cells: FxHashMap<Cell, Vec<u32>>,
    cell_of: Vec<Cell>,
    reach: f64,
}
fn column(x: f64, z: f64) -> Cell {
    ((x / CELL).floor() as i32, (z / CELL).floor() as i32)
}
pub fn corner(b: Aabb) -> Cell {
    column(b.min(Axis::X), b.min(Axis::Z))
}
fn width(b: Aabb) -> f64 {
    (b.max(Axis::X) - b.min(Axis::X)).max(b.max(Axis::Z) - b.min(Axis::Z))
}
fn columns(area: Aabb, reach: f64, count: usize) -> Option<Columns> {
    let margin = reach + 1.0;
    let lo = column(area.min(Axis::X) - margin, area.min(Axis::Z) - margin);
    let hi = column(area.max(Axis::X), area.max(Axis::Z));
    let span = |a: i32, b: i32| i64::from(b) - i64::from(a) + 1;
    let (nx, nz) = (span(lo.0, hi.0), span(lo.1, hi.1));
    (nx >= 1 && nz >= 1 && nx.saturating_mul(nz) <= count as i64).then_some(Columns { lo, hi })
}

impl SeenGrid {
    pub fn new(boxes: impl Iterator<Item = Aabb>) -> Self {
        let mut grid = Self::default();
        for (i, b) in boxes.enumerate() {
            let at = corner(b);
            grid.reach = grid.reach.max(width(b));
            grid.cells.entry(at).or_default().push(i as u32);
            grid.cell_of.push(at);
        }
        grid
    }
    pub fn update(&mut self, i: usize, b: Aabb) {
        self.reach = self.reach.max(width(b));
        let at = corner(b);
        let old = std::mem::replace(&mut self.cell_of[i], at);
        if old == at {
            return;
        }
        if let Some(list) = self.cells.get_mut(&old)
            && let Some(k) = list.iter().position(|&j| j == i as u32)
        {
            list.swap_remove(k);
        }
        self.cells.entry(at).or_default().push(i as u32);
    }
    pub fn columns(&self, area: Aabb) -> Option<Columns> {
        columns(area, self.reach, self.cells.len())
    }
    pub fn candidates(&self, area: Aabb, out: &mut Vec<u32>) -> bool {
        out.clear();
        let Some(columns) = self.columns(area) else {
            return false;
        };
        for cell in columns.cells() {
            if let Some(list) = self.cells.get(&cell) {
                out.extend_from_slice(list);
            }
        }
        out.sort_unstable();
        true
    }
}
impl SnapshotGrid for SeenGrid {
    fn columns(&self, area: Aabb) -> Option<Columns> {
        self.columns(area)
    }
    fn candidates(&self, area: Aabb, out: &mut Vec<u32>) -> bool {
        self.candidates(area, out)
    }
}

/// Region-owned changes over an immutable shared grid. Empty cells are kept,
/// exactly as in SeenGrid::update: their presence affects the scan fallback.
pub struct GridOverlay<'a> {
    base: &'a SeenGrid,
    cells: FxHashMap<Cell, Vec<u32>>,
    cell_of: FxHashMap<usize, Cell>,
    added_cells: usize,
    reach: f64,
}
impl<'a> GridOverlay<'a> {
    pub fn new(base: &'a SeenGrid) -> Self {
        Self {
            base,
            cells: Default::default(),
            cell_of: Default::default(),
            added_cells: 0,
            reach: base.reach,
        }
    }
    fn cell_mut(&mut self, at: Cell) -> &mut Vec<u32> {
        self.cells
            .entry(at)
            .or_insert_with(|| match self.base.cells.get(&at) {
                Some(list) => list.clone(),
                None => {
                    self.added_cells += 1;
                    Vec::new()
                }
            })
    }
    pub fn update(&mut self, slot: usize, bounds: Aabb) {
        self.reach = self.reach.max(width(bounds));
        let at = corner(bounds);
        let old = self
            .cell_of
            .insert(slot, at)
            .unwrap_or(self.base.cell_of[slot]);
        if old == at {
            return;
        }
        let list = self.cell_mut(old);
        if let Some(i) = list.iter().position(|&i| i == slot as u32) {
            list.swap_remove(i);
        }
        self.cell_mut(at).push(slot as u32);
    }
}
impl SnapshotGrid for GridOverlay<'_> {
    fn columns(&self, area: Aabb) -> Option<Columns> {
        columns(area, self.reach, self.base.cells.len() + self.added_cells)
    }
    fn candidates(&self, area: Aabb, out: &mut Vec<u32>) -> bool {
        out.clear();
        let Some(columns) = self.columns(area) else {
            return false;
        };
        for cell in columns.cells() {
            if let Some(list) = self.cells.get(&cell).or_else(|| self.base.cells.get(&cell)) {
                out.extend_from_slice(list);
            }
        }
        out.sort_unstable();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::Vec3;

    fn boxed(x: f64, y: f64, z: f64, w: f64) -> Aabb {
        Aabb::new(Vec3::new(x, y, z), Vec3::new(x + w, y + 1.8, z + w))
    }

    #[test]
    fn candidates_hold_every_meeting_box_in_order() {
        // A small LCG keeps the test free of dependencies.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |range: f64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * range
        };
        let mut boxes: Vec<Aabb> = (0..300)
            .map(|_| {
                boxed(
                    next(64.0) - 32.0,
                    next(8.0),
                    next(64.0) - 32.0,
                    0.3 + next(1.2),
                )
            })
            .collect();
        let mut grid = SeenGrid::new(boxes.iter().copied());
        let mut out = Vec::new();
        let mut indexed = 0;
        for round in 0..2000 {
            // Move one box, sometimes far and wider, as mobs do between queries.
            let i = round % boxes.len();
            let w = if round % 97 == 0 { 3.9 } else { 0.6 };
            boxes[i] = boxed(next(80.0) - 40.0, next(8.0), next(80.0) - 40.0, w);
            grid.update(i, boxes[i]);
            let area = boxed(next(70.0) - 35.0, next(8.0), next(70.0) - 35.0, next(2.0));
            let expected: Vec<u32> = (0..boxes.len() as u32)
                .filter(|&j| boxes[j as usize].intersects(area))
                .collect();
            if grid.candidates(area, &mut out) {
                indexed += 1;
                assert!(out.windows(2).all(|p| p[0] < p[1]), "ascending, no repeats");
                let met: Vec<u32> = out
                    .iter()
                    .copied()
                    .filter(|&j| boxes[j as usize].intersects(area))
                    .collect();
                assert_eq!(met, expected);
            }
        }
        assert!(indexed > 1900, "the grid answered {indexed} of 2000");
    }

    #[test]
    fn sparse_overlay_reproduces_serial_candidates_after_movement_and_resizing() {
        let initial: Vec<_> = (0..240)
            .map(|i| boxed((i % 24) as f64 * 4.0, 0.0, (i / 24) as f64 * 4.0, 0.6))
            .collect();
        let base = SeenGrid::new(initial.iter().copied());
        let mut serial = SeenGrid::new(initial.iter().copied());
        let mut overlay = GridOverlay::new(&base);
        for tick in 0..400 {
            // One owner changes only these slots. Other owners remain borrowed.
            let slot = [0, 19, 43, 118][tick % 4];
            let bounds = boxed(
                (tick % 53) as f64 - 20.0,
                0.0,
                (tick % 37) as f64,
                if tick % 23 == 0 { 9.0 } else { 0.6 },
            );
            serial.update(slot, bounds);
            overlay.update(slot, bounds);
            for range in [1.0, 12.0, 48.0, 900.0] {
                let area = boxed((tick % 41) as f64, 0.0, (tick % 29) as f64, range);
                let (mut expected, mut actual) = (Vec::new(), Vec::new());
                assert_eq!(overlay.columns(area), serial.columns(area));
                assert_eq!(
                    overlay.candidates(area, &mut actual),
                    serial.candidates(area, &mut expected)
                );
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn unbounded_areas_fall_back_to_a_full_scan() {
        let grid = SeenGrid::new([boxed(0.0, 0.0, 0.0, 0.6)].into_iter());
        let mut out = Vec::new();
        let huge = Aabb::new(
            Vec3::new(f64::NEG_INFINITY, 0.0, 0.0),
            Vec3::new(f64::INFINITY, 1.0, 1.0),
        );
        assert!(!grid.candidates(huge, &mut out));
    }
}

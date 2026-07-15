// Uniform-grid spatial index.
//
// Every dobject is bucketed into all cells its bounding box overlaps.
// Queries take a rectangle (or a point + radius) and return a deduplicated
// list of candidate dobject indices — typically O(visible cells × avg per cell)
// instead of O(N).
//
// Bucketing decision: a fixed cell size, chosen by `auto_cell_size` to target
// ~10 dobjects per cell on average. Memory cost is O(N · avg cells per dobject).
//
// Trade-off: this index assumes a roughly uniform dobject distribution. For
// pathological cases (e.g. one giant dobject covering the whole drawing plus a
// million tiny ones) a quadtree would do better. Uniform grid is fast to build,
// trivially parallel, and matches the array-grid workload we want to stress.

use crate::dobject::DObject;
use crate::math::Vec2;

pub struct UniformGrid {
    pub cell_size: f64,
    pub origin:    Vec2,   // world coord of the grid's (0,0) corner
    pub cols:      usize,
    pub rows:      usize,
    cells: Vec<Vec<u32>>,  // row-major, cells[row * cols + col]
    /// Indices of dobjects whose `Geom::is_view_independent_bbox()`
    /// returned true (Hatch today). They're NOT bucketed into cells
    /// because their bbox is a degenerate placeholder; instead they
    /// get appended to every query result so the render path always
    /// has a chance to draw them.
    view_independent: Vec<u32>,
    /// Total dobjects this index was built from. Used by `query_bbox`
    /// to size the visited bitset for O(1) dedup (vs HashSet, which
    /// becomes pathological when entities cover many cells and the
    /// query hits many cells — at 580 cells/entity, a full-grid query
    /// performed 100M HashSet inserts and tanked FPS to ~2).
    n_entities: usize,
}

impl UniformGrid {
    pub fn empty() -> Self {
        Self {
            cell_size: 1.0,
            origin:    Vec2::ZERO,
            cols: 0, rows: 0,
            cells: Vec::new(),
            view_independent: Vec::new(),
            n_entities: 0,
        }
    }

    /// Build the index from `dobjects`. Panics if `cell_size <= 0`.
    pub fn build(dobjects: &[DObject], cell_size: f64) -> Self {
        assert!(cell_size > 0.0, "cell_size must be positive");
        if dobjects.is_empty() { return Self::empty(); }

        // overall world bbox — only over dobjects whose bbox is
        // view-meaningful. View-independent bboxes (e.g. Hatch's
        // placeholder (0,0)) would otherwise distort the grid origin
        // and force absurd cell counts.
        let mut min = Vec2::new(f64::INFINITY, f64::INFINITY);
        let mut max = Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut view_independent: Vec<u32> = Vec::new();
        for (i, e) in dobjects.iter().enumerate() {
            if e.geom.is_view_independent_bbox() {
                view_independent.push(i as u32);
                continue;
            }
            let (emin, emax) = e.bbox();
            if emin.x < min.x { min.x = emin.x; }
            if emin.y < min.y { min.y = emin.y; }
            if emax.x > max.x { max.x = emax.x; }
            if emax.y > max.y { max.y = emax.y; }
        }
        // If EVERY dobject is view-independent, there's no spatial grid
        // to build — return an empty grid that still carries the
        // global list, so query_bbox keeps returning the hatches.
        if !min.x.is_finite() {
            return Self {
                cell_size, origin: Vec2::ZERO, cols: 0, rows: 0,
                cells: Vec::new(),
                view_independent,
                n_entities: dobjects.len(),
            };
        }

        let cols = (((max.x - min.x) / cell_size).floor() as isize + 1).max(1) as usize;
        let rows = (((max.y - min.y) / cell_size).floor() as isize + 1).max(1) as usize;
        let mut cells: Vec<Vec<u32>> = vec![Vec::new(); cols * rows];

        for (i, e) in dobjects.iter().enumerate() {
            if e.geom.is_view_independent_bbox() {
                continue;   // already in view_independent
            }
            let (emin, emax) = e.bbox();
            let xmin = (((emin.x - min.x) / cell_size).floor() as isize).max(0) as usize;
            let xmax = (((emax.x - min.x) / cell_size).floor() as isize).max(0) as usize;
            let ymin = (((emin.y - min.y) / cell_size).floor() as isize).max(0) as usize;
            let ymax = (((emax.y - min.y) / cell_size).floor() as isize).max(0) as usize;
            let xmax = xmax.min(cols - 1);
            let ymax = ymax.min(rows - 1);
            for cy in ymin..=ymax {
                let row_start = cy * cols;
                for cx in xmin..=xmax {
                    cells[row_start + cx].push(i as u32);
                }
            }
        }

        Self { cell_size, origin: min, cols, rows, cells, view_independent,
               n_entities: dobjects.len() }
    }

    /// Pick a cell size that targets `target_per_cell` dobjects per cell, on
    /// average, given the dobjects' overall bbox area.
    pub fn auto_cell_size(dobjects: &[DObject], target_per_cell: f64) -> f64 {
        if dobjects.is_empty() { return 1.0; }
        let mut min = Vec2::new(f64::INFINITY, f64::INFINITY);
        let mut max = Vec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut count_real: usize = 0;
        for e in dobjects {
            if e.geom.is_view_independent_bbox() { continue; }
            count_real += 1;
            let (emin, emax) = e.bbox();
            if emin.x < min.x { min.x = emin.x; }
            if emin.y < min.y { min.y = emin.y; }
            if emax.x > max.x { max.x = emax.x; }
            if emax.y > max.y { max.y = emax.y; }
        }
        if count_real == 0 { return 1.0; }
        let w = (max.x - min.x).max(1.0);
        let h = (max.y - min.y).max(1.0);
        let by_density = (w * h * target_per_cell / count_real as f64).sqrt();
        by_density.max(0.001).min(1.0e6)
    }

    /// Candidate dobject indices whose stored cells overlap the rectangle.
    /// Deduplicated. DObjects were bucketed by bbox so callers may still need
    /// a tighter test for exact-overlap semantics.
    pub fn query_bbox(&self, q_min: Vec2, q_max: Vec2) -> Vec<u32> {
        // Even an empty cell grid must still return the view-independent
        // dobjects (Hatch etc.) — their bbox is a placeholder, so a
        // bbox-overlap test would always miss them.
        if self.cells.is_empty() {
            return self.view_independent.clone();
        }
        let cs = self.cell_size;
        let to_cell = |v: f64, axis_origin: f64| -> isize {
            ((v - axis_origin) / cs).floor() as isize
        };
        let xmin = to_cell(q_min.x, self.origin.x).max(0) as usize;
        let xmax = (to_cell(q_max.x, self.origin.x).max(0) as usize).min(self.cols - 1);
        let ymin = to_cell(q_min.y, self.origin.y).max(0) as usize;
        let ymax = (to_cell(q_max.y, self.origin.y).max(0) as usize).min(self.rows - 1);
        if xmin > xmax || ymin > ymax {
            return self.view_independent.clone();
        }

        // Fast path: when the query covers ≥80% of the grid we'd be
        // iterating most cells anyway. Skip the cell scan, return
        // every entity. Avoids the worst case (zoomed-to-extents on a
        // drawing where entities span many cells — the per-cell visit
        // count alone becomes the bottleneck).
        let cells_q = (xmax - xmin + 1) * (ymax - ymin + 1);
        let cells_t = self.cols * self.rows;
        if cells_t > 0 && (cells_q * 5) >= (cells_t * 4) {
            let mut out: Vec<u32> = (0..self.n_entities as u32).collect();
            // view_independent indices are already in 0..n_entities
            // since they share the same flat index space.
            let _ = &self.view_independent;
            return out;
        }

        // Dedup with a flat Vec<bool> sized n_entities. ~5 ns per
        // mark-and-test vs ~50 ns per HashSet insert — and crucially,
        // no allocator / hash work per duplicate. At 580 cells/entity
        // (zoomed-out array drawings) this is the difference between
        // ~500 ms and ~20 ms per query.
        let mut seen = vec![false; self.n_entities];
        let mut out: Vec<u32> = Vec::with_capacity(256);
        for cy in ymin..=ymax {
            let row = cy * self.cols;
            for cx in xmin..=xmax {
                for &idx in &self.cells[row + cx] {
                    let u = idx as usize;
                    if u < seen.len() && !seen[u] {
                        seen[u] = true;
                        out.push(idx);
                    }
                }
            }
        }
        // Append view-independent always-include set.
        for &idx in &self.view_independent {
            let u = idx as usize;
            if u < seen.len() && !seen[u] {
                seen[u] = true;
                out.push(idx);
            }
        }
        out
    }

    pub fn query_near(&self, p: Vec2, radius: f64) -> Vec<u32> {
        self.query_bbox(
            Vec2::new(p.x - radius, p.y - radius),
            Vec2::new(p.x + radius, p.y + radius),
        )
    }

    /// (total_cells, total_index_entries, cell_size).
    /// `total_index_entries / N_entities` gives the average cells-per-dobject.
    pub fn stats(&self) -> (usize, usize, f64) {
        let total: usize = self.cells.iter().map(|c| c.len()).sum();
        (self.cols * self.rows, total, self.cell_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Circle;

    fn c(x: f64, y: f64, r: f64) -> DObject {
        Circle { center: Vec2::new(x, y), radius: r }.into()
    }

    #[test]
    fn empty_grid_returns_empty() {
        let g = UniformGrid::build(&[], 1.0);
        assert!(g.query_bbox(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0)).is_empty());
    }

    #[test]
    fn basic_query() {
        let ents = vec![
            c(  0.0,  0.0, 1.0),
            c( 10.0, 10.0, 1.0),
            c( 50.0, 50.0, 1.0),
            c(100.0,100.0, 1.0),
        ];
        let g = UniformGrid::build(&ents, 5.0);
        let r = g.query_bbox(Vec2::new(-2.0, -2.0), Vec2::new(12.0, 12.0));
        // Should find indices 0 and 1, not 2 or 3
        assert!(r.contains(&0) && r.contains(&1));
        assert!(!r.contains(&2) && !r.contains(&3));
    }

    #[test]
    fn near_point_query() {
        let ents = vec![
            c( 0.0, 0.0, 1.0),
            c(50.0, 0.0, 1.0),
            c( 0.0,50.0, 1.0),
        ];
        let g = UniformGrid::build(&ents, 5.0);
        let r = g.query_near(Vec2::new(0.0, 0.0), 10.0);
        assert!(r.contains(&0));
        assert!(!r.contains(&1));
        assert!(!r.contains(&2));
    }
}

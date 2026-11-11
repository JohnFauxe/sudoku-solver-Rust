//! Grid topologies. A topology describes a puzzle shape as a set of cells
//! and constraint units: `Classic` is one 9x9 grid (81 cells, 27 units);
//! `Samurai` is five overlapping 9x9 grids laid out in a cross on a 21x21
//! display (369 cells, 135 units — rows/columns/boxes of each grid, where
//! overlapping box cells belong to *both* grids, which is what enforces
//! the Samurai consistency rule).
//!
//! Every engine and both UIs work against a `Topology`, so any shape is
//! solved/rendered without engine-specific code.

/// A cell can belong to at most 3+3 units (two grids' row/col/box).
pub const MAX_UNITS_PER_CELL: usize = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Classic,
    Samurai,
}

#[derive(Clone)]
pub struct Topology {
    pub shape: Shape,
    /// Number of cells (81 / 369).
    pub cells: usize,
    /// Number of constraint units (27 / 135).
    pub units: usize,
    /// Display dimensions (9x9 or 21x21).
    pub w: usize,
    pub h: usize,
    /// display[row][col] -> cell index, or `usize::MAX` for non-cells.
    pub index: Vec<Vec<usize>>,
    /// cell -> display position.
    pub positions: Vec<(usize, usize)>,
    /// cell -> bitmask of grids containing it (bit g = grid g).
    pub grids: Vec<u8>,
    /// cell -> unit indices it participates in (up to 6).
    pub cell_units: Vec<Vec<usize>>,
    /// unit -> member cell indices (exactly 9 cells each).
    pub unit_cells: Vec<Vec<usize>>,
    /// unit -> (grid index, kind) where kind 0=row, 1=col, 2=box.
    pub unit_info: Vec<(u8, u8)>,
    /// cell -> draw a vertical box separator to its left?
    pub left_border: Vec<bool>,
    /// cell -> draw a horizontal box separator above it?
    pub top_border: Vec<bool>,
}

impl Topology {
    pub fn new(shape: Shape) -> Topology {        let offsets: &[(usize, usize)] = match shape {
            Shape::Classic => &[(0, 0)],
            Shape::Samurai => &[(0, 0), (0, 12), (6, 6), (12, 0), (12, 12)],
        };
        let (w, h) = match shape {
            Shape::Classic => (9, 9),
            Shape::Samurai => (21, 21),
        };
        let n_units = 27 * offsets.len();
        let mut t = Topology {
            shape,
            cells: 0,
            units: n_units,
            w,
            h,
            index: vec![vec![usize::MAX; w]; h],
            positions: Vec::new(),
            grids: Vec::new(),
            cell_units: Vec::new(),
            unit_cells: vec![Vec::new(); n_units],
            unit_info: Vec::new(),
            left_border: Vec::new(),
            top_border: Vec::new(),
        };
        for u in 0..n_units {
            t.unit_info.push(((u / 27) as u8, (u % 27 / 9) as u8));
        }

        for (gi, &(gr, gc)) in offsets.iter().enumerate() {
            for i in 0..81usize {
                let dr = gr + i / 9;
                let dc = gc + i % 9;
                let ci = if t.index[dr][dc] == usize::MAX {
                    let ci = t.positions.len();
                    t.index[dr][dc] = ci;
                    t.positions.push((dr, dc));
                    t.grids.push(0);
                    t.cell_units.push(Vec::new());
                    t.left_border.push(false);
                    t.top_border.push(false);
                    ci
                } else {
                    t.index[dr][dc]
                };
                t.grids[ci] |= 1 << gi;

                // Box separator borders, per host grid.
                let (ir, ic) = (i / 9, i % 9);
                if ir % 3 == 0 && ir != 0 {
                    t.top_border[ci] = true;
                }
                if ic % 3 == 0 && ic != 0 {
                    t.left_border[ci] = true;
                }

                // Membership in this grid's row / column / box units.
                let base = gi * 27;
                t.link(base + ir, ci);
                t.link(base + 9 + ic, ci);
                t.link(base + 18 + (ir / 3) * 3 + ic / 3, ci);
            }
        }
        t.cells = t.positions.len();
        t
    }

    fn link(&mut self, unit: usize, cell: usize) {
        debug_assert!(self.cell_units[cell].len() < MAX_UNITS_PER_CELL);
        self.unit_cells[unit].push(cell);
        self.cell_units[cell].push(unit);
    }

    /// Human name of a unit for error messages, e.g. "grid 3, row 4".
    pub fn unit_name(&self, unit: usize) -> String {
        let (g, kind) = self.unit_info[unit];
        let kind = match kind {
            0 => "row",
            1 => "column",
            _ => "box",
        };
        if self.units == 27 {
            format!("{kind} {}", unit % 27 % 9 + 1)
        } else {
            format!("grid {}, {} {}", g + 1, kind, unit % 27 % 9 + 1)
        }
    }

    /// Display columns that are box separators for at least one grid
    /// (any cell in the column carries a left border).
    pub fn pipe_columns(&self) -> Vec<bool> {
        let mut pipes = vec![false; self.w];
        for (&(r, c), idx) in self.positions.iter().zip(0..) {
            let _ = r;
            if self.left_border[idx] {
                pipes[c] = true;
            }
        }
        pipes
    }

    /// Display rows that have a horizontal box separator.
    pub fn separator_rows(&self) -> Vec<bool> {
        let mut seps = vec![false; self.h];
        for (&(r, _c), idx) in self.positions.iter().zip(0..) {
            if self.top_border[idx] {
                seps[r] = true;
            }
        }
        seps
    }
}

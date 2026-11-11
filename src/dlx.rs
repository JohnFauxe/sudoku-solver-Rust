//! Knuth's Algorithm X with Dancing Links (DLX), generalized over any
//! topology.
//!
//! Exact-cover columns, each of which must be satisfied exactly once:
//!   1..=cells        every cell holds exactly one digit
//!   cells+1 ..       for every unit and digit: the unit contains digit d
//!
//! Rows = one per (cell, digit) possibility, pruned by the puzzle's clues
//! (a given cell keeps only its own digit's row, which is pre-selected —
//! its columns covered — before the search starts).
//!
//! Search order is Knuth's S heuristic: always branch on the column with
//! the fewest remaining rows. Every row selection/deselection is reported
//! through the same [`StepLogger`] hook the other engines use, so all UIs
//! animate DLX solving too, including its backtracking.

use crate::grid::Grid;
use crate::solver::{
    NopLogger, Placer, SolveConfig, SolveOutcome, SolveStats, StepInfo, StepLogger,
};
use crate::topo::Topology;

struct Dlx {
    // Dancing-links node arrays; index 0 is the root, 1.. are headers.
    l: Vec<usize>,
    r: Vec<usize>,
    u: Vec<usize>,
    d: Vec<usize>,
    col: Vec<usize>,    // column header of a node (== itself for headers)
    row_of: Vec<usize>, // placement id (cell*9 + d-1), usize::MAX for headers
    sizes: [u32; 1 << 12],
    // placement id -> its node in the cell-constraint column
    cell_node: Vec<usize>,
    cells: Vec<u8>,
    top: Topology,
}

/// Column header (array index) of the cell constraint for `cell`.
fn cell_header(cell: usize) -> usize {
    1 + cell
}

/// Column header (array index) of the "unit u contains digit d" constraint.
fn unit_header(top: &Topology, unit: usize, d: u8) -> usize {
    1 + top.cells + unit * 9 + (d as usize - 1)
}

impl Dlx {
    fn new(grid: &Grid) -> Dlx {
        let top = grid.top.clone();
        // Number of column headers (excluding the root): one per cell,
        // plus one per (unit, digit) pair.
        let headers = top.cells + top.units * 9;
        let mut s = Dlx {
            l: Vec::new(),
            r: Vec::new(),
            u: Vec::new(),
            d: Vec::new(),
            col: Vec::new(),
            row_of: Vec::new(),
            sizes: [0; 1 << 12],
            cell_node: vec![0; top.cells * 9],
            cells: grid.cells().to_vec(),
            top: top.clone(),
        };
        assert!(headers < s.sizes.len(), "topology too large for DLX");
        // Root.
        s.push_node(0, usize::MAX);
        // Column headers, linked in a circle with the root.
        for ci in 1..=headers {
            s.add_header(ci);
        }
        // Placements. A clue cell keeps only its own digit's row.
        for cell in 0..top.cells {
            let clue = grid.cells()[cell];
            for d in 1u8..=9 {
                if clue == 0 || d == clue {
                    s.add_row(&top, cell, d);
                }
            }
        }
        // Pre-select clue rows.
        for cell in 0..top.cells {
            let d = grid.cells()[cell];
            if d != 0 {
                s.cover_clue(cell, d);
            }
        }
        s
    }

    fn push_node(&mut self, col: usize, row_of: usize) -> usize {
        let n = self.l.len();
        self.l.push(0);
        self.r.push(0);
        self.u.push(0);
        self.d.push(0);
        self.col.push(col);
        self.row_of.push(row_of);
        n
    }

    fn add_header(&mut self, ci: usize) {
        let prev = self.l[0];
        let n = self.push_node(ci, usize::MAX);
        self.r[prev] = n;
        self.l[n] = prev;
        self.r[n] = 0;
        self.l[0] = n;
        self.u[n] = n;
        self.d[n] = n;
        self.sizes[ci] = 0;
    }

    fn add_row(&mut self, top: &Topology, cell: usize, d: u8) {
        let mut cols = vec![cell_header(cell)];
        for &unit in &top.cell_units[cell] {
            cols.push(unit_header(top, unit, d));
        }
        let row = cell * 9 + d as usize - 1;
        let mut first = usize::MAX;
        let mut prev = usize::MAX;
        for &ci in &cols {
            let n = self.push_node(ci, row);
            // Append at the bottom of the column's circular list.
            let bot = self.u[ci];
            self.u[n] = bot;
            self.d[n] = ci;
            self.d[bot] = n;
            self.u[ci] = n;
            self.sizes[ci] += 1;
            if prev == usize::MAX {
                first = n;
            } else {
                self.r[prev] = n;
                self.l[n] = prev;
            }
            prev = n;
        }
        // Close the row's horizontal circle.
        self.r[prev] = first;
        self.l[first] = prev;
        self.cell_node[row] = first;
    }

    /// Remove (hide) a column and every row touching it.
    fn cover(&mut self, ci: usize) {
        let l = self.l[ci];
        let rt = self.r[ci];
        self.r[l] = rt;
        self.l[rt] = l;
        let mut i = self.d[ci];
        while i != ci {
            let mut j = self.r[i];
            while j != i {
                self.d[self.u[j]] = self.d[j];
                self.u[self.d[j]] = self.u[j];
                self.sizes[self.col[j]] -= 1;
                j = self.r[j];
            }
            i = self.d[i];
        }
    }

    /// Exact inverse of `cover`.
    fn uncover(&mut self, ci: usize) {
        let mut i = self.u[ci];
        while i != ci {
            let mut j = self.l[i];
            while j != i {
                self.sizes[self.col[j]] += 1;
                self.d[self.u[j]] = j;
                self.u[self.d[j]] = j;
                j = self.l[j];
            }
            i = self.u[i];
        }
        let l = self.l[ci];
        let rt = self.r[ci];
        self.r[l] = ci;
        self.l[rt] = ci;
    }

    /// Select a clue's row before searching; no event is emitted since the
    /// UIs already display the imported puzzle.
    fn cover_clue(&mut self, cell: usize, d: u8) {
        let start = self.cell_node[cell * 9 + d as usize - 1];
        self.cover(self.col[start]);
        let mut j = self.r[start];
        while j != start {
            self.cover(self.col[j]);
            j = self.r[j];
        }
    }

    /// Recurse through Algorithm X. `solutions` collects up to
    /// `cfg.max_solutions` completed grids.
    fn search(
        &mut self,
        solutions: &mut Vec<Grid>,
        cfg: SolveConfig,
        stats: &mut SolveStats,
        vis: &mut dyn StepLogger,
    ) {
        if solutions.len() >= cfg.max_solutions {
            return;
        }
        if self.r[0] == 0 {
            // All constraints satisfied.
            solutions.push(Grid::from_cells(self.top.clone(), self.cells.clone()));
            return;
        }
        // S heuristic: branch on the smallest active column.
        let mut ci = self.r[0];
        let mut c = ci;
        let mut best = self.sizes[ci];
        while c != 0 {
            if self.sizes[c] < best {
                best = self.sizes[c];
                ci = c;
            }
            c = self.r[c];
        }
        if best == 0 {
            return; // a constraint with no rows left: dead end
        }
        self.cover(ci);
        let mut i = self.d[ci];
        while i != ci && solutions.len() < cfg.max_solutions {
            // Try row i: assign its cell, hide all conflicting rows.
            let cell = self.row_of[i] / 9;
            let num = self.row_of[i] % 9;
            let dgt = num as u8 + 1;
            self.cells[cell] = dgt;
            stats.guesses += 1;
            vis.step(&self.cells, Some(StepInfo { idx: cell, digit: dgt, placer: Placer::Dlx }));
            let mut j = self.r[i];
            while j != i {
                self.cover(self.col[j]);
                j = self.r[j];
            }
            self.search(solutions, cfg, stats, vis);
            if solutions.len() < cfg.max_solutions {
                // Untry the row: uncover its columns, clear the cell.
                let mut k = self.l[i];
                while k != i {
                    self.uncover(self.col[k]);
                    k = self.l[k];
                }
                self.cells[cell] = 0;
                stats.backtracks += 1;
                vis.step(&self.cells, Some(StepInfo { idx: cell, digit: 0, placer: Placer::Undo }));
                i = self.d[i];
            }
        }
        if solutions.len() < cfg.max_solutions {
            self.uncover(ci);
        }
    }
}

/// Solve `grid` with Dancing Links.
pub fn solve_dlx(
    grid: &Grid,
    cfg: SolveConfig,
    logger: Option<&mut dyn StepLogger>,
) -> SolveOutcome {
    let mut dlx = Dlx::new(grid);
    let mut stats = SolveStats::default();
    let mut solutions: Vec<Grid> = Vec::new();
    match logger {
        None => dlx.search(&mut solutions, cfg, &mut stats, &mut NopLogger),
        Some(l) => dlx.search(&mut solutions, cfg, &mut stats, l),
    }
    match solutions.len() {
        0 => SolveOutcome::NoSolution { stats },
        1 => SolveOutcome::Unique {
            solution: solutions.remove(0),
            stats,
        },
        _ => SolveOutcome::Multiple { solutions, stats },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;
    use crate::solver::{Algorithm, Placer};
    use crate::topo::{Shape, Topology};

    const WIKIPEDIA: &str = concat!(
        "530070000", "600195000", "098000060",
        "800060003", "400803001", "700020006",
        "060000280", "000419005", "000080079",
    );

    /// Arto Inkala's 2019 "world's hardest" puzzle.
    const INKALA: &str = concat!(
        "8........", "..36.....", ".7..9.2..",
        ".5...7...", "....457..", "...1...3.",
        "..1....68", "..85...1.", ".9....4..",
    );

    fn dlx_cfg(max: usize) -> SolveConfig {
        SolveConfig { algorithm: Algorithm::DancingLinks, ..SolveConfig::with_max(max) }
    }

    #[test]
    fn solves_easy_puzzle_the_same_as_mrv() {
        let g = Grid::parse(WIKIPEDIA).unwrap();
        match solve_dlx(&g, dlx_cfg(2), None) {
            SolveOutcome::Unique { solution, stats } => {
                assert_eq!(solution.to_compact(), concat!(
                    "534678912", "672195348", "198342567",
                    "859761423", "426853791", "713924856",
                    "961537284", "287419635", "345286179",
                ));
                assert!(stats.guesses > 0);
                assert_eq!(stats.naked_singles, 0);
                assert_eq!(stats.hidden_singles, 0);
            }
            other => panic!("expected unique solution, got {other:?}"),
        }
    }

    #[test]
    fn solves_the_hardest_puzzle_of_2019() {
        let g = Grid::parse(INKALA).unwrap();
        match solve_dlx(&g, dlx_cfg(2), None) {
            SolveOutcome::Unique { solution, .. } => {
                assert!(solution.is_complete_and_valid());
                assert!(solution.cells()[0] == 8); // clue preserved
            }
            other => panic!("expected unique solution, got {other:?}"),
        }
    }

    #[test]
    fn detects_no_solution() {
        let g = Grid::parse(concat!(
            "1........", "2........", "3........",
            "4........", "5........", "6........",
            "7........", "8........", ".9.......",
        )).unwrap();
        assert!(matches!(
            solve_dlx(&g, dlx_cfg(2), None),
            SolveOutcome::NoSolution { .. }
        ));
    }

    #[test]
    fn detects_multiple_solutions() {
        // A single clue leaves the grid massively under-constrained.
        let puzzle = "5".to_string() + &".".repeat(80);
        let g = Grid::parse(&puzzle).unwrap();
        match solve_dlx(&g, dlx_cfg(2), None) {
            SolveOutcome::Multiple { solutions, .. } => {
                assert_eq!(solutions.len(), 2);
                assert!(solutions[0].is_complete_and_valid());
                assert!(solutions[1].is_complete_and_valid());
                assert_ne!(solutions[0].cells(), solutions[1].cells());
            }
            other => panic!("expected multiple solutions, got {other:?}"),
        }
    }

    #[test]
    fn logs_selections_and_undos() {
        struct L {
            events: u64,
            undos: u64,
            last_cells: Vec<u8>,
        }
        impl StepLogger for L {
            fn step(&mut self, cells: &[u8], info: Option<StepInfo>) {
                self.events += 1;
                if matches!(info, Some(StepInfo { placer: Placer::Undo, .. })) {
                    self.undos += 1;
                }
                self.last_cells = cells.to_vec();
            }
        }
        // An under-constrained puzzle makes DLX backtrack, so undo events
        // must appear.
        let puzzle = "5".to_string() + &".".repeat(80);
        let g = Grid::parse(&puzzle).unwrap();
        let mut l = L { events: 0, undos: 0, last_cells: Vec::new() };
        match solve_dlx(&g, dlx_cfg(2), Some(&mut l)) {
            SolveOutcome::Multiple { .. } => {
                assert!(l.events > 0);
                assert!(l.undos > 0, "expected backtrack (undo) events");
            }
            other => panic!("expected multiple solutions, got {other:?}"),
        }
    }

    #[test]
    fn never_places_outside_allowed_digits() {
        // Cell clues must survive: the solution contains every clue as-is.
        let g = Grid::parse(WIKIPEDIA).unwrap();
        if let SolveOutcome::Unique { solution, .. } = solve_dlx(&g, dlx_cfg(1), None) {
            for i in 0..81 {
                let clue = g.cells()[i];
                if clue != 0 {
                    assert_eq!(solution.cells()[i], clue, "clue cell {i} changed");
                }
            }
        } else {
            panic!("expected unique solution");
        }
    }

    // ----- Samurai -----

    fn samurai_solution() -> Vec<u8> {
        let empty = Grid::new(Topology::new(Shape::Samurai));
        match solve_dlx(&empty, SolveConfig::with_max(1), None) {
            SolveOutcome::Unique { solution, .. } => solution.cells().to_vec(),
            other => panic!("empty samurai must be solvable, got {other:?}"),
        }
    }

    #[test]
    fn solves_samurai() {
        let sol = samurai_solution();
        let top = Topology::new(Shape::Samurai);
        // Keep every third cell as a clue; DLX must complete it.
        let puzzle: Vec<u8> = sol
            .iter()
            .enumerate()
            .map(|(i, &d)| if i % 3 == 0 { d } else { 0 })
            .collect();
        let g = Grid::from_cells(top, puzzle.clone());
        match solve_dlx(&g, dlx_cfg(1), None) {
            SolveOutcome::Unique { solution, .. } => {
                assert!(solution.is_complete_and_valid());
                for (i, &p) in puzzle.iter().enumerate() {
                    if p != 0 {
                        assert_eq!(solution.cells()[i], p);
                    }
                }
            }
            other => panic!("expected samurai solution, got {other:?}"),
        }
    }

    #[test]
    fn empty_samurai_has_many_solutions() {
        let empty = Grid::new(Topology::new(Shape::Samurai));
        assert!(matches!(
            solve_dlx(&empty, dlx_cfg(2), None),
            SolveOutcome::Multiple { .. }
        ));
    }
}

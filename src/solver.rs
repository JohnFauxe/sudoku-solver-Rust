//! The solving engine.
//!
//! Strategy (per search node):
//!   1. Constraint propagation: repeatedly apply
//!        - naked singles  (a cell whose candidate list has one digit)
//!        - hidden singles (a digit that fits only one cell in a unit)
//!      until a fixed point, contradiction, or completion.
//!   2. If empty cells remain, branch on the most-constrained empty cell
//!      (MRV: minimum remaining values) and recurse.
//!
//! Candidate bookkeeping is three groups of bitmasks (`used`): one bitmask
//! per row, column and box, where bit `d - 1` is set if digit `d` is used.
//! The candidate set of a cell is `FULL & !(used[row] | used[col] | used[box])`.

use std::fmt;

use crate::grid::{unit_cell, Grid, FULL};

/// 27 units: 0..9 rows, 9..18 columns, 18..27 boxes.
const UNIT_COUNT: usize = 27;

#[derive(Clone, Copy, Default, Debug)]
pub struct SolveStats {
    pub naked_singles: u32,
    pub hidden_singles: u32,
    pub guesses: u32,
    pub backtracks: u32,
}

impl SolveStats {
    /// Rough difficulty tier from how much work was needed.
    pub fn difficulty(&self) -> &'static str {
        if self.guesses == 0 {
            if self.hidden_singles == 0 {
                "Easy"
            } else {
                "Medium"
            }
        } else {
            "Hard"
        }
    }
}

impl fmt::Display for SolveStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "naked singles: {}, hidden singles: {}, search nodes: {}, dead ends: {}",
            self.naked_singles, self.hidden_singles, self.guesses, self.backtracks
        )
    }
}

#[derive(Clone)]
struct Solver {
    cells: [u8; 81],
    used: [u16; UNIT_COUNT],
}

impl Solver {
    /// Assumes `grid` has already been clue-validated.
    fn new(grid: &Grid) -> Solver {
        let mut s = Solver { cells: [0; 81], used: [0; UNIT_COUNT] };
        for idx in 0..81 {
            let d = grid.cells()[idx];
            if d != 0 {
                s.place(idx, d);
            }
        }
        s
    }

    fn place(&mut self, idx: usize, d: u8) {
        let bit = 1u16 << (d - 1);
        self.cells[idx] = d;
        let r = idx / 9;
        let c = idx % 9;
        self.used[r] |= bit;
        self.used[9 + c] |= bit;
        self.used[18 + crate::grid::box_of(r, c)] |= bit;
    }

    /// Candidate digit bitmask for an (empty) cell.
    fn candidates(&self, idx: usize) -> u16 {
        let r = idx / 9;
        let c = idx % 9;
        FULL & !(self.used[r] | self.used[9 + c] | self.used[18 + crate::grid::box_of(r, c)])
    }

    /// Repeatedly fill in all naked and hidden singles until nothing changes.
    /// Returns `false` on contradiction (an empty cell has no candidates,
    /// or some digit has nowhere to go in a unit).
    fn propagate(&mut self, stats: &mut SolveStats, vis: &mut dyn StepLogger) -> bool {
        let mut placements: Vec<(usize, u8)> = Vec::with_capacity(9);

        loop {
            let mut progress = false;

            // Naked singles: a cell with exactly one candidate.
            for idx in 0..81 {
                if self.cells[idx] != 0 {
                    continue;
                }
                let m = self.candidates(idx);
                if m == 0 {
                    return false;
                }
                if m.count_ones() == 1 {
                    let d = m.trailing_zeros() as u8 + 1;
                    self.place(idx, d);
                    stats.naked_singles += 1;
                    progress = true;
                    vis.step(&self.cells, Some(StepInfo { idx, digit: d, placer: Placer::Naked }));
                }
            }

            // Hidden singles: for every unit and every missing digit there,
            // find the cells it can go; if exactly one, place it.
            let mut contradiction = false;
            for unit in 0..UNIT_COUNT {
                for digit in 1u8..=9 {
                    let bit = 1u16 << (digit - 1);
                    if self.used[unit] & bit != 0 {
                        continue;
                    }
                    let mut target: Option<usize> = None;
                    let mut count = 0;
                    for i in 0..9 {
                        let idx = unit_cell(unit, i);
                        if self.cells[idx] == 0 && self.candidates(idx) & bit != 0 {
                            count += 1;
                            if count > 1 {
                                break;
                            }
                            target = Some(idx);
                        }
                    }
                    match (count, target) {
                        (0, _) => {
                            // A digit with no home in this unit.
                            contradiction = true;
                        }
                        (1, Some(idx)) => placements.push((idx, digit)),
                        _ => {}
                    }
                }
            }
            if contradiction {
                return false;
            }

            // Apply the collected placements defensively: another placement
            // from this batch may already have filled the cell or used the
            // digit in one of its units.
            for (idx, digit) in placements.drain(..) {
                let bit = 1u16 << (digit - 1);
                if self.cells[idx] == 0 && self.candidates(idx) & bit != 0 {
                    self.place(idx, digit);
                    stats.hidden_singles += 1;
                    progress = true;
                    vis.step(&self.cells, Some(StepInfo { idx, digit, placer: Placer::Hidden }));
                }
            }

            if !progress {
                return true;
            }
        }
    }

    /// The empty cell with fewest candidates. After a successful
    /// `propagate`, every empty cell has at least one candidate.
    fn mrv_cell(&self) -> Option<(usize, u16)> {
        let mut best: Option<(usize, u16, u32)> = None; // (idx, mask, popcount)
        for idx in 0..81 {
            if self.cells[idx] != 0 {
                continue;
            }
            let m = self.candidates(idx);
            let n = m.count_ones();
            match best {
                Some((_, _, bn)) if bn <= n => {}
                _ => best = Some((idx, m, n)),
            }
        }
        best.map(|(idx, m, _)| (idx, m))
    }

    /// Depth-first search producing up to `max` solutions.
    fn search(
        &mut self,
        out: &mut Vec<Grid>,
        cfg: SolveConfig,
        stats: &mut SolveStats,
        vis: &mut dyn StepLogger,
    ) {
        if out.len() >= cfg.max_solutions {
            return;
        }
        if !self.propagate(stats, &mut *vis) {
            stats.backtracks += 1;
            return;
        }
        match self.mrv_cell() {
            None => out.push(Grid::from_cells(self.cells)),
            Some((idx, mask)) => {
                if !cfg.allow_search {
                    // Singles-only mode: propagate has stalled; do not guess.
                    return;
                }
                stats.guesses += 1;
                let solved_before = out.len();
                let mut m = mask;
                while m != 0 {
                    let bit = m & m.wrapping_neg(); // lowest set bit
                    m &= m - 1;
                    let d = bit.trailing_zeros() as u8 + 1;
                    let mut branch = self.clone();
                    branch.place(idx, d);
                    vis.step(&branch.cells, Some(StepInfo { idx, digit: d, placer: Placer::Guess }));
                    branch.search(out, cfg, stats, &mut *vis);
                }
                if out.len() == solved_before {
                    stats.backtracks += 1;
                }
            }
        }
    }
}

/// Who made a placement during a step-logging solve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placer {
    /// Naked single: cell with one candidate.
    Naked,
    /// Hidden single: digit fitting only one cell in a unit.
    Hidden,
    /// Search hypothesis (later branches may undo it).
    Guess,
    /// Dancing-links row selection.
    Dlx,
    /// A dancing-links row was deselected (backtracked); the cell is empty.
    Undo,
}

/// Which solving engine to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    /// Constraint propagation (naked + hidden singles) + MRV backtracking.
    Propagation,
    /// Knuth's Algorithm X with dancing links (`dlx`).
    DancingLinks,
}

/// Knobs the caller can turn.
#[derive(Clone, Copy, Debug)]
pub struct SolveConfig {
    /// Stop once this many solutions are found (>= 1). 2 proves uniqueness.
    pub max_solutions: usize,
    /// If false, the solver stops as soon as pure singles logic stalls
    /// (no hypothesis search / backtracking). Ignored by the DLX engine.
    pub allow_search: bool,
    /// Which solving engine to run.
    pub algorithm: Algorithm,
}

impl Default for SolveConfig {
    fn default() -> Self {
        SolveConfig {
            max_solutions: 2,
            allow_search: true,
            algorithm: Algorithm::Propagation,
        }
    }
}

impl SolveConfig {
    /// Legacy behaviour: search enabled, stop after `max` solutions.
    pub fn with_max(max_solutions: usize) -> SolveConfig {
        SolveConfig { max_solutions, ..SolveConfig::default() }
    }

    /// Builder-style: pick the engine.
    pub fn with_algorithm(mut self, algorithm: Algorithm) -> SolveConfig {
        self.algorithm = algorithm;
        self
    }
}

/// What the last placement was.
#[derive(Clone, Copy, Debug)]
pub struct StepInfo {
    pub idx: usize,
    pub digit: u8,
    /// Kept for API completeness; the plain solver path never inspects it.
    #[allow(dead_code)]
    pub placer: Placer,
}

/// Receives the full cell array after every placement, in solver order.
pub trait StepLogger {
    fn step(&mut self, cells: &[u8; 81], info: Option<StepInfo>);
}

/// Discards step events (used when nobody is watching).
pub(crate) struct NopLogger;

impl StepLogger for NopLogger {
    fn step(&mut self, _cells: &[u8; 81], _info: Option<StepInfo>) {}
}

/// Solve `grid`, stopping once `max_solutions` solutions are found.
/// If `logger` is given, it is called for every placement as it happens,
/// including placements that are later undone by backtracking.
pub fn solve_with(
    grid: &Grid,
    cfg: SolveConfig,
    logger: Option<&mut dyn StepLogger>,
) -> SolveOutcome {
    match cfg.algorithm {
        Algorithm::Propagation => solve_propagation(grid, cfg, logger),
        Algorithm::DancingLinks => crate::dlx::solve_dlx(grid, cfg, logger),
    }
}

/// The propagation + MRV engine (formerly the whole solver).
fn solve_propagation(
    grid: &Grid,
    cfg: SolveConfig,
    logger: Option<&mut dyn StepLogger>,
) -> SolveOutcome {
    let mut solver = Solver::new(grid);
    let mut stats = SolveStats::default();
    let mut solutions: Vec<Grid> = Vec::new();
    match logger {
        None => solver.search(&mut solutions, cfg, &mut stats, &mut NopLogger),
        Some(l) => solver.search(&mut solutions, cfg, &mut stats, l),
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

/// Outcome of a solving request.
#[derive(Clone)]
pub enum SolveOutcome {
    /// No grid satisfies the clues.
    NoSolution { stats: SolveStats },
    /// Exactly one solution found (search stopped at `max_solutions` found).
    Unique { solution: Grid, stats: SolveStats },
    /// At least `solutions.len()` different solutions exist.
    Multiple { solutions: Vec<Grid>, stats: SolveStats },
}

impl fmt::Debug for SolveOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolveOutcome::NoSolution { stats } => write!(f, "NoSolution({stats:?})"),
            SolveOutcome::Unique { solution, stats } => {
                write!(f, "Unique({:?}, {:?})", solution.to_compact(), stats)
            }
            SolveOutcome::Multiple { solutions, stats } => write!(
                f,
                "Multiple({:?}, {:?})",
                solutions.iter().map(Grid::to_compact).collect::<Vec<_>>(),
                stats
            ),
        }
    }
}

/// Solve `grid`, stopping once `max_solutions` solutions are found.
/// `max_solutions` of 2 is enough to detect non-unique puzzles.
/// (Convenience wrapper; the CLIs build a `SolveConfig` directly.)
#[allow(dead_code)]
pub fn solve(grid: &Grid, max_solutions: usize) -> SolveOutcome {
    solve_with(grid, SolveConfig::with_max(max_solutions), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;

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

    /// Provably unsolvable: rows 1-8 of column 1 contain 1..=8 and row 9
    /// contains a 9 in column 2, leaving cell (9,1) zero candidates.
    const UNSOLVABLE: &str = concat!(
        "1........", "2........", "3........",
        "4........", "5........", "6........",
        "7........", "8........", ".9.......",
    );

    #[test]
    fn solves_easy_puzzle_uniquely() {
        let g = Grid::parse(WIKIPEDIA).unwrap();
        let expected = concat!(
            "534678912", "672195348", "198342567",
            "859761423", "426853791", "713924856",
            "961537284", "287419635", "345286179",
        );
        match solve(&g, 2) {
            SolveOutcome::Unique { solution, stats } => {
                assert_eq!(solution.to_compact(), expected);
                // The classic example needs a few hidden singles but no search.
                assert_eq!(stats.difficulty(), "Medium");
                assert_eq!(stats.guesses, 0, "wikipedia puzzle needs no search");
            }
            other => panic!("expected unique solution, got {other:?}"),
        }
    }

    #[test]
    fn solves_hardest_puzzle_of_2019() {
        let g = Grid::parse(INKALA).unwrap();
        match solve(&g, 2) {
            SolveOutcome::Unique { solution, stats } => {
                assert!(solution.is_complete_and_valid());
                assert_eq!(solution.get(0, 0), 8); // clue preserved
                assert_eq!(stats.difficulty(), "Hard");
            }
            other => panic!("expected unique solution, got {other:?}"),
        }
    }

    #[test]
    fn detects_no_solution() {
        let g = Grid::parse(UNSOLVABLE).unwrap();
        match solve(&g, 2) {
            SolveOutcome::NoSolution { stats } => assert!(stats.backtracks > 0),
            other => panic!("expected no solution, got {other:?}"),
        }
    }

    #[test]
    fn detects_multiple_solutions() {
        // A single clue leaves the grid massively under-constrained.
        let puzzle = "5".to_string() + &".".repeat(80);
        let g = Grid::parse(&puzzle).unwrap();
        match solve(&g, 2) {
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
    fn solve_with_logs_steps_to_the_solution() {
        struct L {
            events: u64,
            guess_events: u64,
            first_full: Option<[u8; 81]>,
        }
        impl StepLogger for L {
            fn step(&mut self, cells: &[u8; 81], info: Option<StepInfo>) {
                self.events += 1;
                if matches!(info, Some(StepInfo { placer: Placer::Guess, .. })) {
                    self.guess_events += 1;
                }
                // With max > 1 the search keeps running after the first
                // solution to prove uniqueness, so remember the first
                // completed board instead of the last event.
                if self.first_full.is_none() && cells.iter().all(|&d| d != 0) {
                    self.first_full = Some(*cells);
                }
            }
        }

        // The 2019 Inkala puzzle needs search, so guess/undo events must appear.
        let g = Grid::parse(INKALA).unwrap();
        let mut l = L { events: 0, guess_events: 0, first_full: None };
        let outcome = solve_with(&g, SolveConfig::with_max(2), Some(&mut l));
        match outcome {
            SolveOutcome::Unique { solution, .. } => {
                assert!(l.events > 0);
                assert!(l.guess_events > 0, "a search-heavy puzzle must emit guess events");
                // The first fully-populated board is exactly the solution.
                assert_eq!(l.first_full.unwrap(), *solution.cells());
            }
            other => panic!("expected unique solution, got {other:?}"),
        }
    }

    #[test]
    fn solve_without_logger_matches_plain_solve() {
        let g = Grid::parse(WIKIPEDIA).unwrap();
        match solve(&g, 2) {
            SolveOutcome::Unique { solution: a, .. } => match solve_with(&g, SolveConfig::with_max(2), None) {
                SolveOutcome::Unique { solution: b, .. } => {
                    assert_eq!(a.cells(), b.cells());
                }
                other => panic!("expected unique, got {other:?}"),
            },
            other => panic!("expected unique, got {other:?}"),
        }
    }

    #[test]
    fn config_can_forbid_search() {
        // Singles-only mode: Inkala stalls, so no solution is reached.
        let g = Grid::parse(INKALA).unwrap();
        let cfg = SolveConfig { max_solutions: 2, allow_search: false, ..SolveConfig::default() };
        assert!(matches!(
            solve_with(&g, cfg, None),
            SolveOutcome::NoSolution { .. }
        ));

        // The easy puzzle only needs singles, so it still solves uniquely.
        let g = Grid::parse(WIKIPEDIA).unwrap();
        assert!(matches!(
            solve_with(&g, cfg, None),
            SolveOutcome::Unique { .. }
        ));
    }

    #[test]
    fn config_max_one_stops_at_first_solution() {
        let g = Grid::parse(WIKIPEDIA).unwrap();
        let cfg = SolveConfig { max_solutions: 1, allow_search: true, ..SolveConfig::default() };
        match solve_with(&g, cfg, None) {
            SolveOutcome::Unique { solution, .. } => {
                assert_eq!(solution.get(0, 0), 5);
            }
            other => panic!("expected unique solution, got {other:?}"),
        }
    }

    #[test]
    fn empty_grid_has_many_solutions() {        let g = Grid::empty();
        assert!(matches!(solve(&g, 2), SolveOutcome::Multiple { .. }));
    }
}

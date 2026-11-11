//! Naive backtracking engine: fill the first empty cell with digits 1..=9
//! in order, step back on contradiction. No candidate tracking, no
//! heuristics — the baseline the other engines are benchmarked against.
//!
//! It emits the same step events as the other engines (placements and
//! undos), so the terminal watch mode and browser UI animate it too.

use crate::grid::Grid;
use crate::solver::{NopLogger, Placer, SolveConfig, SolveOutcome, SolveStats, StepInfo, StepLogger};

/// True if digit `d` at cell `idx` conflicts with no value in its row,
/// column or box in `cells`.
fn allowed(cells: &[u8; 81], idx: usize, d: u8) -> bool {
    let (r, c) = (idx / 9, idx % 9);
    for k in 0..9 {
        if cells[r * 9 + k] == d || cells[k * 9 + c] == d {
            return false;
        }
    }
    let (br, bc) = ((r / 3) * 3, (c / 3) * 3);
    for rr in br..br + 3 {
        for cc in bc..bc + 3 {
            if cells[rr * 9 + cc] == d {
                return false;
            }
        }
    }
    true
}

/// Depth-first from the first empty cell at/after `idx`.
fn search(
    cells: &mut [u8; 81],
    idx: usize,
    cfg: SolveConfig,
    stats: &mut SolveStats,
    solutions: &mut Vec<Grid>,
    vis: &mut dyn StepLogger,
) {
    if solutions.len() >= cfg.max_solutions {
        return;
    }
    let mut i = idx;
    while i < 81 && cells[i] != 0 {
        i += 1;
    }
    if i == 81 {
        // No empty cells: this assignment is a complete solution.
        solutions.push(Grid::from_cells(*cells));
        return;
    }
    for d in 1u8..=9 {
        if !allowed(cells, i, d) {
            continue;
        }
        cells[i] = d;
        stats.guesses += 1;
        vis.step(cells, Some(StepInfo { idx: i, digit: d, placer: Placer::Naive }));
        search(cells, i + 1, cfg, stats, solutions, vis);
        if solutions.len() >= cfg.max_solutions {
            // Keep the winning assignment on the board and unwind for good.
            return;
        }
        cells[i] = 0;
        stats.backtracks += 1;
        vis.step(cells, Some(StepInfo { idx: i, digit: 0, placer: Placer::Undo }));
    }
}

/// Solve `grid` with naive backtracking.
pub fn solve_naive(
    grid: &Grid,
    cfg: SolveConfig,
    logger: Option<&mut dyn StepLogger>,
) -> SolveOutcome {
    let mut cells = *grid.cells();
    let mut stats = SolveStats::default();
    let mut solutions: Vec<Grid> = Vec::new();
    match logger {
        None => search(&mut cells, 0, cfg, &mut stats, &mut solutions, &mut NopLogger),
        Some(l) => search(&mut cells, 0, cfg, &mut stats, &mut solutions, l),
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
    use crate::solver::{Algorithm, SolveConfig};

    const WIKIPEDIA: &str = concat!(
        "530070000", "600195000", "098000060",
        "800060003", "400803001", "700020006",
        "060000280", "000419005", "000080079",
    );

    /// Unsolvable with a contradiction at the *second* empty cell in scan
    /// order: cell (0,2) is attacked by its row ({1,2,9}), its column
    /// ({3..9}) and its box ({1,2,3,9?}) — naive discovers it instantly.
    /// (The generic unsolvable test grid would take naive eons.)
    const CONTRADICTION_EARLY: &str = concat!(
        "12.......", "..3......", "..4......",
        "..5......", "..6......", "..7......",
        "..8......", ".........", "..9......",
    );

    fn naive_cfg(max: usize) -> SolveConfig {
        SolveConfig { algorithm: Algorithm::NaiveBacktracking, ..SolveConfig::with_max(max) }
    }

    #[test]
    fn solves_easy_puzzle() {
        let g = Grid::parse(WIKIPEDIA).unwrap();
        match solve_naive(&g, naive_cfg(1), None) {
            SolveOutcome::Unique { solution, stats } => {
                assert!(solution.is_complete_and_valid());
                assert!(stats.guesses > 0);
                assert!(stats.backtracks > 0); // even easy puzzles make it hesitate
                assert_eq!(stats.naked_singles, 0);
            }
            other => panic!("expected unique solution, got {other:?}"),
        }
    }

    #[test]
    fn preserves_clues() {
        let g = Grid::parse(WIKIPEDIA).unwrap();
        if let SolveOutcome::Unique { solution, .. } = solve_naive(&g, naive_cfg(1), None) {
            for r in 0..9 {
                for c in 0..9 {
                    let clue = g.get(r, c);
                    if clue != 0 {
                        assert_eq!(solution.get(r, c), clue);
                    }
                }
            }
        } else {
            panic!("expected unique solution");
        }
    }

    #[test]
    fn detects_no_solution() {
        let g = Grid::parse(CONTRADICTION_EARLY).unwrap();
        assert!(matches!(
            solve_naive(&g, naive_cfg(2), None),
            SolveOutcome::NoSolution { .. }
        ));
    }

    #[test]
    fn detects_multiple_solutions() {
        let puzzle = "5".to_string() + &".".repeat(80);
        let g = Grid::parse(&puzzle).unwrap();
        match solve_naive(&g, naive_cfg(2), None) {
            SolveOutcome::Multiple { solutions, .. } => {
                assert_eq!(solutions.len(), 2);
                assert!(solutions[0].is_complete_and_valid());
                assert_ne!(solutions[0].cells(), solutions[1].cells());
            }
            other => panic!("expected multiple solutions, got {other:?}"),
        }
    }

    #[test]
    fn logs_placements_and_undos() {
        struct L {
            events: u64,
            undos: u64,
            placements: u64,
        }
        impl StepLogger for L {
            fn step(&mut self, _cells: &[u8; 81], info: Option<StepInfo>) {
                self.events += 1;
                match info {
                    Some(StepInfo { placer: Placer::Undo, .. }) => self.undos += 1,
                    Some(StepInfo { placer: Placer::Naive, .. }) => self.placements += 1,
                    _ => {}
                }
            }
        }
        let g = Grid::parse(WIKIPEDIA).unwrap();
        let mut l = L { events: 0, undos: 0, placements: 0 };
        match solve_naive(&g, naive_cfg(1), Some(&mut l)) {
            SolveOutcome::Unique { .. } => {
                assert!(l.events > 0);
                assert!(l.placements > 0);
                assert!(l.undos > 0);
            }
            other => panic!("expected unique solution, got {other:?}"),
        }
    }
}

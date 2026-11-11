//! `--bench` mode: race every solving engine against one puzzle.
//!
//! Each engine runs a warmup solve (doubling as the baseline measurement),
//! then doubling batches of solves until a batch takes at least
//! `MIN_BATCH`. Per-solve time = latest batch time / batch size. A
//! wall-clock watchdog abandons an engine whose single solve takes too
//! long (naive backtracking will happily run for hours on hard puzzles;
//! its measurement thread dies with the process on exit).

use std::time::{Duration, Instant};
use std::thread;

use crate::grid::Grid;
use crate::solver::{solve_with, Algorithm, SolveConfig, SolveOutcome};

/// Keep measuring until one batch takes at least this long.
const MIN_BATCH: Duration = Duration::from_millis(200);
/// Never grow a batch beyond this many solves.
const MAX_ITERS: u64 = 1 << 22;
/// Watchdog: how long a single warmup solve may take before the engine is
/// reported as abandoned.
const WATCHDOG: Duration = Duration::from_secs(30);

pub struct EngineBench {
    pub name: &'static str,
    /// Kept for API completeness; `name` is the printable form.
    #[allow(dead_code)]
    pub algorithm: Algorithm,
    /// Solves in the measured batch (1 if the warmup solve alone passed
    /// the threshold, or if it was abandoned).
    pub iters: u64,
    /// Duration of that batch.
    pub total: Duration,
    /// Outcome of the warmup solve (used for the correctness check).
    pub outcome: SolveOutcome,
    /// True if the warmup solve exceeded the watchdog.
    pub abandoned: bool,
}

fn engines() -> [(&'static str, Algorithm); 3] {
    [
        ("mrv", Algorithm::Propagation),
        ("dlx", Algorithm::DancingLinks),
        ("naive", Algorithm::NaiveBacktracking),
    ]
}

/// Measure one engine. Runs in the calling thread; the abandoned case is
/// only triggered via a watchdog thread wrapping this in `run`.
fn measure(grid: &Grid, name: &'static str, algorithm: Algorithm, min_batch: Duration, max_iters: u64) -> EngineBench {
    let cfg = SolveConfig::with_max(2).with_algorithm(algorithm);
    let t0 = Instant::now();
    let outcome = solve_with(grid, cfg, None);
    let mut total = t0.elapsed();
    let mut iters: u64 = 1;
    while total < min_batch && iters < max_iters {
        let batch = iters; // double: next batch solves 2x
        let t = Instant::now();
        for _ in 0..batch {
            let _ = solve_with(grid, cfg, None);
        }
        total = t.elapsed();
        iters *= 2;
    }
    EngineBench { name, algorithm, iters, total, outcome, abandoned: false }
}

pub fn bench_engines(grid: &Grid, min_batch: Duration, max_iters: u64, watchdog: Duration) -> Vec<EngineBench> {
    let mut results = Vec::new();
    for (name, algorithm) in engines() {
        let grid_copy = *grid;
        let (tx, rx) = std::sync::mpsc::channel();
        let t0 = Instant::now();
        thread::spawn(move || {
            let r = measure(&grid_copy, name, algorithm, min_batch, max_iters);
            let _ = tx.send(r);
        });
        let result = match rx.recv_timeout(watchdog) {
            Ok(r) => r,
            Err(_) => {
                // The spawned thread keeps spinning until the process
                // exits; record the abandonment.
                EngineBench {
                    name,
                    algorithm,
                    iters: 0,
                    total: t0.elapsed(),
                    outcome: SolveOutcome::NoSolution { stats: Default::default() },
                    abandoned: true,
                }
            }
        };
        results.push(result);
        // Let the OS settle before measuring the next engine.
        thread::sleep(Duration::from_millis(20));
    }
    results
}

/// True if every clue of `grid` is preserved in `solution` and the grid is
/// complete and valid.
fn consistent(solution: &Grid, puzzle: &Grid) -> bool {
    if !solution.is_complete_and_valid() {
        return false;
    }
    for i in 0..81 {
        let clue = puzzle.cells()[i];
        if clue != 0 && solution.cells()[i] != clue {
            return false;
        }
    }
    true
}

pub fn run(grid: &Grid) -> crate::Exit {
    crate::out(std::format_args!(
        "benchmark: {} clues, target = unique/uniqueness proof (stop after 2)\n",
        grid.cells().iter().filter(|&&d| d != 0).count()
    ));

    let results = bench_engines(grid, MIN_BATCH, MAX_ITERS, WATCHDOG);

    crate::out(std::format_args!(
        "{:<8} {:>10} {:>14} {:>10}   {}",
        "engine", "solves", "per solve", "total", "work per solve"
    ));

    for r in &results {
        let stats_line = match &r.outcome {
            SolveOutcome::Unique { stats, .. } => format!(
                "{} naked + {} hidden singles, {} placements, {} dead ends",
                stats.naked_singles, stats.hidden_singles, stats.guesses, stats.backtracks
            ),
            SolveOutcome::Multiple { stats, .. } => format!(
                "multiple (2+): {} placements, {} dead ends", stats.guesses, stats.backtracks
            ),
            SolveOutcome::NoSolution { stats } => format!(
                "no solution: {} placements, {} dead ends", stats.guesses, stats.backtracks
            ),
        };
        if r.abandoned {
            crate::out(std::format_args!(
                "{:<8} {:>10} {:>14} {:>10}   ABANDONED after {:.0}s (still spinning)",
                r.name, "-", "-", "-", r.total.as_secs_f32()
            ));
            continue;
        }
        let per_solve = r.total / (r.iters.max(1) as u32);
        let per = if per_solve.as_millis() >= 1 {
            format!("{:.2} ms", per_solve.as_secs_f64() * 1e3)
        } else {
            format!("{:.1} µs", per_solve.as_secs_f64() * 1e6)
        };
        crate::out(std::format_args!(
            "{:<8} {:>10} {:>14} {:>10}   {}",
            r.name,
            r.iters,
            per,
            format!("{:.0} ms", r.total.as_secs_f64() * 1e3),
            stats_line,
        ));
    }

    // Correctness cross-check: every engine that finished must have
    // produced consistent, valid solutions.
    let mut ok = true;
    for r in &results {
        match &r.outcome {
            SolveOutcome::Unique { solution, .. } => {
                if !consistent(solution, grid) {
                    ok = false;
                    crate::out(std::format_args!(
                        "CHECK FAILED: {} returned an inconsistent/invalid grid",
                        r.name
                    ));
                }
            }
            SolveOutcome::Multiple { solutions, .. } => {
                for s in solutions {
                    if !consistent(s, grid) {
                        ok = false;
                    }
                }
            }
            SolveOutcome::NoSolution { .. } => {}
        }
    }
    if ok {
        crate::out(std::format_args!("correctness: every finished engine produced consistent solution(s) OK"));
    }
    crate::OK
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

    #[test]
    fn all_engines_agree_on_unique_puzzle() {        let g = Grid::parse(WIKIPEDIA).unwrap();
        let results = bench_engines(&g, Duration::ZERO, 1, WATCHDOG);
        assert_eq!(results.len(), 3);
        let mut solutions = Vec::new();
        for r in &results {
            assert!(!r.abandoned);
            match &r.outcome {
                SolveOutcome::Unique { solution, .. } => {
                    assert!(consistent(solution, &g));
                    solutions.push(solution.to_compact());
                }
                other => panic!("expected unique from {}, got {other:?}", r.name),
            }
        }
        assert_eq!(solutions[0], solutions[1]);
        assert_eq!(solutions[1], solutions[2]);
    }
}

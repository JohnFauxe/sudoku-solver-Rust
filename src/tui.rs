//! Terminal "watch" UI: displays the imported puzzle, then animates the
//! solver populating the grid as it works. Every placement triggers a
//! screen update (cell colors mark fresh moves and backtracked cells),
//! throttled to a target frame rate so hard puzzles animate too.
//!
//! Works for any topology: classic 9x9 and Samurai 21x21 (five overlapping
//! grids with holes between them).
//!
//! Pure ANSI escapes, zero dependencies. `Ctrl-C` simply terminates; the
//! terminal keeps its last frame.

use std::fmt::Write as _;
use std::io::{self, Write as IoWrite};
use std::time::{Duration, Instant};

use crate::grid::{cell_char, Grid, Shape};
use crate::solver::{Algorithm, SolveConfig, SolveOutcome, SolveStats, StepInfo, StepLogger};
use crate::{Exit, FAIL, MULTI, OK};

/// Counts solver steps without touching the screen; used to size the
/// animation so it stays watchable (we cannot know the step count up front).
struct CountLogger(u64);

impl StepLogger for CountLogger {
    fn step(&mut self, _cells: &[u8], _info: Option<StepInfo>) {
        self.0 += 1;
    }
}

struct Watch {
    grid: Grid,
    clues: Vec<u8>,
    shown: Vec<u8>,
    last: Option<StepInfo>,
    events: u64,
    /// Consider only every N-th solver event as a frame candidate, so the
    /// total animation length stays roughly constant for huge searches.
    step_every: u64,
    interval: Duration,
    next_frame: Option<Instant>,
    /// Column is a box separator for some grid.
    pipe_col: Vec<bool>,
    /// Row has a horizontal box separator above it.
    sep_row: Vec<bool>,
}

impl Watch {
    fn style(&self, idx: usize, d: u8) -> &'static str {
        if self.clues[idx] != 0 {
            return "\x1b[1;97m"; // bold white: original clue
        }
        if let Some(info) = &self.last {
            if info.idx == idx && info.digit == d && d != 0 {
                return "\x1b[1;96m"; // bright cyan: most recent move
            }
        }
        if self.shown[idx] != d {
            if d == 0 {
                return "\x1b[90m"; // dim gray: just undone / backtracked
            }
            return "\x1b[1;92m"; // bright green: freshly placed
        }
        if d != 0 {
            return "\x1b[97m"; // white: settled
        }
        ""
    }

    /// Redraw the whole frame. Cursor is sent home first so nothing flickers;
    /// `\x1b[K`/`\x1b[J` erase stale pixels on shorter frames.
    fn render(&mut self, cells: &[u8], footer: &str) {
        let (w, h) = (self.grid.top.w, self.grid.top.h);
        let mut s = String::with_capacity(16384);
        let _ = writeln!(
            s,
            "\x1b[H\x1b[1;35mSudoku Solver\x1b[0m — watch mode   events: {}  \x1b[K\n",
            self.events
        );
        for r in 0..h {
            // Horizontal box separator above this row?
            if self.sep_row[r] {
                for c in 0..w {
                    let above = r > 0 && self.grid.top.index[r - 1][c] != usize::MAX;
                    let below = self.grid.top.index[r][c] != usize::MAX;
                    if above || below {
                        let cross = self.pipe_col[c];
                        let _ = write!(s, "{}", if cross { '+' } else { '-' });
                    } else {
                        let _ = write!(s, " ");
                    }
                }
                let _ = writeln!(s, "\x1b[K");
            }
            for c in 0..w {
                let idx = self.grid.top.index[r][c];
                // Vertical box separators draw through every row of the
                // column (also where the column is a hole) so rows align.
                if self.pipe_col[c] {
                    let has_side = (c > 0 && self.grid.top.index[r][c - 1] != usize::MAX)
                        || idx != usize::MAX;
                    let _ = write!(s, "{}", if has_side { '|' } else { ' ' });
                }
                if idx == usize::MAX {
                    let _ = write!(s, "  ");
                    continue;
                }
                let d = cells[idx];
                let ch = cell_char(d);
                let _ = write!(s, "{}{ch}\x1b[0m ", self.style(idx, d));
            }
            let _ = writeln!(s, "\x1b[K");
        }
        if !footer.is_empty() {
            let _ = writeln!(s, "{footer}\x1b[K");
        }
        let _ = write!(s, "\x1b[J");

        let stdout = io::stdout();
        let mut lock = stdout.lock();
        let _ = lock.write_all(s.as_bytes()); // ignore broken pipes
        let _ = lock.flush();
        self.shown = cells.to_vec();
    }

    /// Final frame: static colors only (no "fresh move" highlighting).
    fn render_final(&mut self, cells: &[u8], footer: &str) {
        self.last = None;
        self.shown = cells.to_vec();
        self.render(cells, footer);
    }
}

impl StepLogger for Watch {
    fn step(&mut self, cells: &[u8], info: Option<StepInfo>) {
        self.events += 1;
        if self.events % self.step_every != 0 {
            return; // skip instantly; pacing comes from rendered frames
        }
        self.last = info;
        // Throttle to the requested frame rate.
        if let Some(next) = self.next_frame {
            let now = Instant::now();
            if now < next {
                std::thread::sleep(next - now);
            }
        }
        self.next_frame = Some(Instant::now() + self.interval);
        self.render(cells, "");
    }
}

/// Watch a single puzzle solve, live.
pub fn watch(puzzle: &str, fps: u32, algorithm: Algorithm, shape: Option<Shape>) -> Exit {
    let grid = match Grid::parse_shape(puzzle, shape) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: bad puzzle: {e}");
            return FAIL;
        }
    };
    let fps = fps.clamp(1, 240) as f64;

    // Dry run to count solver events, so we can pick a frame stride that
    // keeps the whole animation in a watchable window (1–10 s).
    let mut count = CountLogger(0);
    let _ = crate::solver::solve_with(&grid, SolveConfig::with_max(2).with_algorithm(algorithm), Some(&mut count));
    let n = count.0 as f64;
    let target = (n / fps).clamp(1.0, 10.0);
    let step_every = ((n / (target * fps)).ceil() as u64).max(1);

    let mut w = Watch {
        grid: grid.clone(),
        clues: grid.cells().to_vec(),
        shown: vec![0; grid.top.cells],
        last: None,
        events: 0,
        step_every,
        interval: Duration::from_secs_f64(1.0 / fps),
        next_frame: None,
        pipe_col: grid.top.pipe_columns(),
        sep_row: grid.top.separator_rows(),
    };

    let mut stdout = io::stdout();
    let _ = stdout.write_all(b"\x1b[2J"); // clear once
    let _ = stdout.flush();

    let clue_count = w.clues.iter().filter(|&&d| d != 0).count();
    let start_footer = format!(
        "imported puzzle — {clue_count} clues; animating ~{n:.0} solver steps at {fps:.0} fps"
    );
    w.render(grid.cells(), &start_footer);
    std::thread::sleep(Duration::from_millis(700));

    let outcome = crate::solver::solve_with(&grid, SolveConfig::with_max(2).with_algorithm(algorithm), Some(&mut w));

    match outcome {
        SolveOutcome::Unique { solution, stats } => {
            let SolveStats { naked_singles, hidden_singles, guesses, backtracks } = stats;
            let tier = match algorithm {
                Algorithm::Propagation => stats.difficulty().to_string(),
                Algorithm::DancingLinks => "dancing links".to_string(),
                Algorithm::NaiveBacktracking => "naive backtracking".to_string(),
            };
            w.render_final(
                solution.cells(),
                &format!(
                    "SOLVED ({}): {naked_singles} naked + {hidden_singles} hidden singles, \
                     {guesses} placements, {backtracks} dead ends",
                    tier
                ),
            );
            OK
        }
        SolveOutcome::NoSolution { .. } => {
            w.render_final(grid.cells(), "NO SOLUTION — no valid grid satisfies these clues");
            FAIL
        }
        SolveOutcome::Multiple { solutions, stats } => {
            w.render_final(
                grid.cells(),
                &format!(
                    "MULTIPLE SOLUTIONS ({} found, capped) — this is not a proper sudoku \
                     ({} placements)",
                    solutions.len(), stats.guesses
                ),
            );
            MULTI
        }
    }
}

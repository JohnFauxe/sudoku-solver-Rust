//! Terminal "watch" UI: displays the imported puzzle, then animates the
//! solver populating the grid as it works. Every placement triggers a
//! screen update (cell colors mark fresh moves and backtracked cells),
//! throttled to a target frame rate so hard puzzles animate too.
//!
//! Pure ANSI escapes, zero dependencies. `Ctrl-C` simply terminates; the
//! terminal keeps its last frame.

use std::fmt::Write as _;
use std::io::{self, Write as IoWrite};
use std::time::{Duration, Instant};

use crate::grid::Grid;
use crate::solver::{SolveOutcome, SolveStats, StepInfo, StepLogger};
use crate::{Exit, FAIL, MULTI, OK};

/// Counts solver steps without touching the screen; used to size the
/// animation so it stays watchable (we cannot know the step count up front).
struct CountLogger(u64);

impl StepLogger for CountLogger {
    fn step(&mut self, _cells: &[u8; 81], _info: Option<StepInfo>) {
        self.0 += 1;
    }
}

struct Watch {
    clues: [u8; 81],
    shown: [u8; 81],
    last: Option<StepInfo>,
    events: u64,
    /// Consider only every N-th solver event as a frame candidate, so the
    /// total animation length stays roughly constant for huge searches.
    step_every: u64,
    interval: Duration,
    next_frame: Option<Instant>,
}

impl Watch {
    fn style(&self, idx: usize, d: u8) -> &'static str {
        if self.clues[idx] != 0 {
            return "\x1b[1;97m"; // bold white: original clue
        }
        if let Some(info) = &self.last {
            if info.idx == idx && info.digit == d {
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
    fn render(&mut self, cells: &[u8; 81], footer: &str) {
        let mut s = String::with_capacity(8192);
        let _ = writeln!(
            s,
            "\x1b[H\x1b[1;35msudoku\x1b[0m — watch mode   events: {}  \x1b[K\n",
            self.events
        );
        for r in 0..9 {
            if r == 3 || r == 6 {
                let _ = writeln!(s, "------+-------+------\x1b[K");
            }
            for c in 0..9 {
                if c == 3 || c == 6 {
                    let _ = write!(s, "| ");
                }
                let idx = r * 9 + c;
                let d = cells[idx];
                let ch = if d == 0 {
                    '.'
                } else {
                    char::from_digit(d as u32, 10).unwrap()
                };
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
        self.shown.copy_from_slice(cells);
    }

    /// Final frame: static colors only (no "fresh move" highlighting).
    fn render_final(&mut self, cells: &[u8; 81], footer: &str) {
        self.last = None;
        self.shown.copy_from_slice(cells);
        self.render(cells, footer);
    }
}

impl StepLogger for Watch {
    fn step(&mut self, cells: &[u8; 81], info: Option<StepInfo>) {
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
pub fn watch(puzzle: &str, fps: u32) -> Exit {
    let grid = match Grid::parse(puzzle) {
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
    let _ = crate::solver::solve_with(&grid, 2, Some(&mut count));
    let n = count.0 as f64;
    let target = (n / fps).clamp(1.0, 10.0);
    let step_every = ((n / (target * fps)).ceil() as u64).max(1);

    let clues = *grid.cells();
    let mut w = Watch {
        clues,
        shown: [0; 81],
        last: None,
        events: 0,
        step_every,
        interval: Duration::from_secs_f64(1.0 / fps),
        next_frame: None,
    };

    let mut stdout = io::stdout();
    let _ = stdout.write_all(b"\x1b[2J"); // clear once
    let _ = stdout.flush();

    let clue_count = clues.iter().filter(|&&d| d != 0).count();
    let start_footer = format!(
        "imported puzzle — {clue_count} clues; animating ~{n:.0} solver steps at {fps:.0} fps"
    );
    w.render(grid.cells(), &start_footer);
    std::thread::sleep(Duration::from_millis(700));

    let outcome = crate::solver::solve_with(&grid, 2, Some(&mut w));

    match outcome {
        SolveOutcome::Unique { solution, stats } => {
            let SolveStats { naked_singles, hidden_singles, guesses, backtracks } = stats;
            w.render_final(
                solution.cells(),
                &format!(
                    "SOLVED ({}): {naked_singles} naked + {hidden_singles} hidden singles, \
                     {guesses} search nodes, {backtracks} dead ends",
                    stats.difficulty()
                ),
            );
            OK
        }
        SolveOutcome::NoSolution => {
            w.render_final(grid.cells(), "NO SOLUTION — no valid grid satisfies these clues");
            FAIL
        }
        SolveOutcome::Multiple { solutions } => {
            w.render_final(
                grid.cells(),
                &format!(
                    "MULTIPLE SOLUTIONS ({} found, capped) — this is not a proper sudoku",
                    solutions.len()
                ),
            );
            MULTI
        }
    }
}

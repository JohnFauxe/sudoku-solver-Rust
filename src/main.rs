//! CLI front-end. Zero dependencies, hand-rolled argument parsing.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;

mod grid;
mod solver;
mod tui;

use grid::Grid;
use solver::{SolveOutcome, SolveStats};

const USAGE: &str = "\
 sudoku — a Sudoku solver (constraint propagation + MRV backtracking)

USAGE:
  sudoku <PUZZLE>           Solve one puzzle: 81 characters, digits 1-9 with
                            0 or . for empty cells, e.g.
                            530070000600195000098000060800060003400803001700020006060000280000419005000080079
                            Drawn grids (spaces and | - + separators) are accepted too.
  sudoku -w --watch <PUZZLE>  Live view: shows the imported puzzle, then
                              animates the grid as the solver fills it in.
  sudoku --fps=<N> <PUZZLE>   Animation speed for --watch, frames per
                              second (default 30).
  sudoku -f <PATH>          Solve every puzzle in a file, one per line.
  sudoku --file=<PATH>      Blank lines and lines starting with '#' are skipped.
  sudoku -h | --help        Show this help.

DIFFICULTY (rough):
  Easy    - solved with naked singles only
  Medium  - needed hidden singles, but no search
  Hard    - needed hypothesis search / backtracking

EXIT CODE:
  0   solved with a unique solution
  1   no solution, or an error (bad input, unreadable file)
  2   multiple solutions (the puzzle is not a proper sudoku)
";

/// Process exit code, chosen so scripts can react to outcome classes:
/// 0 = unique solution, 1 = no solution or error, 2 = multiple solutions.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Exit(i32);

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

const OK: Exit = Exit(0);
const FAIL: Exit = Exit(1);
const MULTI: Exit = Exit(2);
const USAGE_ERROR: Exit = Exit(64);

/// Print to stdout, ignoring write errors (e.g. the pipe closed early when
/// piped into `head`), so a broken pipe never becomes a panic.
fn out(args: fmt::Arguments<'_>) {
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{args}");
    let _ = stdout.flush();
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ExitCode::from(run(&args).0 as u8)
}

fn run(args: &[String]) -> Exit {
    let mut watch = false;
    let mut fps: u32 = 30;
    let mut puzzle: Option<String> = None;
    let mut file: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "-h" | "--help" | "help" => {
                out(format_args!("{USAGE}"));
                return OK;
            }
            "-w" | "--watch" => watch = true,
            "--fps" => match args.get(i + 1).and_then(|v| v.parse::<u32>().ok()) {
                Some(v) => {
                    fps = v;
                    i += 1;
                }
                None => {
                    eprintln!("error: --fps needs a number (frames per second)");
                    eprint!("{USAGE}");
                    return USAGE_ERROR;
                }
            },
            v if v.starts_with("--fps=") => match v.trim_start_matches("--fps=").parse::<u32>() {
                Ok(v) => fps = v,
                Err(_) => {
                    eprintln!("error: bad value for --fps: {v}");
                    eprint!("{USAGE}");
                    return USAGE_ERROR;
                }
            },
            "-f" | "--file" => match args.get(i + 1) {
                Some(p) => {
                    file = Some(p.clone());
                    i += 1;
                }
                None => {
                    eprintln!("error: -f/--file needs a path argument");
                    eprint!("{USAGE}");
                    return USAGE_ERROR;
                }
            },
            v if v.starts_with("--file=") => {
                file = Some(v.trim_start_matches("--file=").to_string())
            }
            v if v.starts_with('-') => {
                eprintln!("error: unknown flag {v}");
                eprint!("{USAGE}");
                return USAGE_ERROR;
            }
            _ => {
                if puzzle.is_some() {
                    eprintln!("error: multiple puzzle arguments");
                    eprint!("{USAGE}");
                    return USAGE_ERROR;
                }
                puzzle = Some(a.clone());
            }
        }
        i += 1;
    }

    match (puzzle, file) {
        (Some(p), None) if watch => tui::watch(&p, fps),
        (Some(p), None) => solve_one(&p),
        (None, Some(f)) => {
            if watch {
                eprintln!("error: --watch animates a single puzzle argument, not a file");
                return USAGE_ERROR;
            }
            solve_file(&f)
        }
        (None, None) => {
            eprintln!("error: expected a puzzle or -f <PATH>");
            eprint!("{USAGE}");
            USAGE_ERROR
        }
        (Some(_), Some(_)) => {
            eprintln!("error: give either a puzzle string or -f <PATH>, not both");
            eprint!("{USAGE}");
            USAGE_ERROR
        }
    }
}

fn solve_one(puzzle: &str) -> Exit {
    let grid = match Grid::parse(puzzle) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: bad puzzle: {e}");
            return FAIL;
        }
    };

    out(format_args!("Puzzle:\n{grid}"));
    report_solve(&grid)
}

fn solve_file(path: &str) -> Exit {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            return FAIL;
        }
    };

    let mut worst = OK;
    for (n, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match Grid::parse(line) {
            Ok(grid) => {
                out(format_args!("\nline {}: {line}\n{grid}", n + 1));
                worst = worst.max(report_solve(&grid));
            }
            Err(e) => {
                out(format_args!("\nline {}: {line}\n  ERROR: {e}", n + 1));
                worst = worst.max(FAIL);
            }
        }
    }
    worst
}

/// Solve and print the result; returns the matching exit code.
fn report_solve(grid: &Grid) -> Exit {
    match solver::solve(grid, 2) {
        SolveOutcome::Unique { solution, stats } => {
            let SolveStats { naked_singles, hidden_singles, guesses, backtracks } = stats;
            out(format_args!(
                "Solution ({}):\n{solution}\n\
                 solver: {naked_singles} naked + {hidden_singles} hidden singles, \
                 {guesses} search nodes, {backtracks} dead ends",
                stats.difficulty(),
            ));
            OK
        }
        SolveOutcome::NoSolution => {
            out(format_args!("Result: NO SOLUTION — no valid grid satisfies these clues."));
            FAIL
        }
        SolveOutcome::Multiple { solutions } => {
            out(format_args!(
                "Result: MULTIPLE SOLUTIONS ({} found, capped) — \
                 this is not a proper sudoku.",
                solutions.len()
            ));
            if let Some(first) = solutions.first() {
                out(format_args!("Example:\n{first}"));
            }
            MULTI
        }
    }
}

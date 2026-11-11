# sudoku

A fast Sudoku solver in Rust with no dependencies.

Strategy: constraint propagation (naked + hidden singles) until a fixed
point, then backtracking on the most-constrained cell (MRV).
Arto Inkala's 2019 "world's hardest" puzzle solves in ~5 ms.

## Build

```sh
cargo build --release
```

## Usage

```sh
# Solve one puzzle: 81 characters, 1-9 = clues, 0 or . = empty
./target/release/sudoku 530070000600195000098000060800060003400803001700020006060000280000419005000080079

# Drawn grids (spaces and | - + ) are accepted as input too
./target/release/sudoku "5 3 . | . 7 . | . . .
6 . . | 1 9 5 | . . .
. 9 8 | . . . | . 6 .
------+-------+------
8 . . | . 6 . | . . 3
4 . . | 8 . 3 | . . 1
7 . . | . 2 . | . . 6
------+-------+------
. 6 . | . . . | 2 8 .
. . . | 4 1 9 | . . 5
. . . | . 8 . | . 7 9"

# Choose the solving engine
./target/release/sudoku --dlx <PUZZLE>       # Knuth's dancing links (Algorithm X)
./target/release/sudoku --naive <PUZZLE>     # plain cell-by-cell backtracking
./target/release/sudoku --algorithm=mrv <PUZZLE>   # default: propagation + MRV

# Benchmark: race all engines on one puzzle, print per-solve timings
./target/release/sudoku --bench <PUZZLE>

# Watch mode: live view of the solver populating the grid
# (clues stay bold white; fresh moves in green, latest in cyan,
#  backtracked cells flash gray; animation stays within ~1-10 s)
./target/release/sudoku -w --fps=30 <PUZZLE>

# Browser UI: import puzzles, configure the solver, watch it solve.
# Opens your default browser at http://127.0.0.1:8173 (Ctrl-C stops it).
./target/release/sudoku --ui
./target/release/sudoku --ui --port=9000 --no-open

# Solve every puzzle in a file, one per line (# comments allowed)
./target/release/sudoku -f puzzles.txt

./target/release/sudoku --help
```

### The browser window (`--ui`)

- **Import**: paste a puzzle (compact or drawn grid), "open file…",
  or click an example. `/​?puzzle=<81 chars>` deep-links too.
- **Configure**: engine (propagation + MRV or dancing links), stop
  after N solutions (1 = first found, 2 = prove uniqueness), and
  search on/off — "singles only" runs stall on hard puzzles by design.
- **Watch**: the engine streams every placement (including search
  guesses and DLX selections that are later undone); the page replays
  it on a colored 9x9 grid with play / pause / step / restart, a speed
  slider (1–500 events/s), a scrubber, a progress bar, and the
  solver's stats and verdict (unique / no solution / multiple / stuck).

### Solving algorithms

| engine | model | notes |
|--------|-------|-------|
| `mrv`  | bitmask candidates: naked + hidden singles, then MRV backtracking | reports which technique placed each cell; "singles only" mode available |
| `dlx`  | exact cover: 324 constraints (cell/row/col/box x digit), 729 placement rows, dancing links with Knuth's S heuristic | every row selection/deselection is a step; typically ~2x faster end-to-end |
| `naive` | cell-by-cell first-fit backtracking, no heuristics | the benchmark baseline; can explode on hard/unsolvable puzzles |

All three engines emit the same step-event stream, so the terminal `--watch`
mode and the browser window animate either engine identically.
Use `--bench` to race them on a puzzle — timings, work counts and a
correctness cross-check between engines.

## Exit codes

| code | meaning                                             |
|------|-----------------------------------------------------|
| 0    | solved with a unique solution                       |
| 1    | no solution, invalid input, or unreadable file      |
| 2    | multiple solutions — the puzzle is not a proper sudoku |
| 64   | usage error                                         |

## Layout

- `src/grid.rs` — grid type, parsing, clue validation (duplicates in
  rows, columns and boxes), pretty printing
- `src/solver.rs` — propagation + MRV engine, `SolveConfig`/`Algorithm`,
  step-event logging, unit tests
- `src/dlx.rs` — dancing links engine: exact-cover model, cover/uncover,
  S heuristic, clue pre-coverage, unit tests
- `src/naive.rs` — naive engine: cell-by-cell first-fit backtracking
- `src/bench.rs` — `--bench`: adaptive-timing race of all engines with
  correctness cross-check
- `src/tui.rs` — `--watch` live view: ANSI screen, step throttling,
  outcome footers
- `src/ui.rs` + `src/ui_page.html` — `--ui` browser window: std-only
  local web server, embedded page, JSON step-event API
- `src/main.rs` — CLI

## Tests

```sh
cargo test
```

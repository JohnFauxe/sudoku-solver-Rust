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

# Watch mode: live view of the solver populating the grid
# (clues stay bold white; fresh moves in green, latest in cyan,
#  backtracked cells flash gray; animation stays within ~1-10 s)
./target/release/sudoku -w --fps=30 <PUZZLE>

# Solve every puzzle in a file, one per line (# comments allowed)
./target/release/sudoku -f puzzles.txt

./target/release/sudoku --help
```

Output includes a rough difficulty tier and solver statistics.

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
- `src/solver.rs` — bitmask solver engine, propagation, MRV search,
  difficulty tiers, step-event logging, unit tests
- `src/tui.rs` — `--watch` live view: ANSI screen, step throttling,
  outcome footers
- `src/main.rs` — CLI

## Tests

```sh
cargo test
```

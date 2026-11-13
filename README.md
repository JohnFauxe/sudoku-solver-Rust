# Sudoku Solver

A fast Sudoku solver written in Rust. It handles classic 9x9 and Samurai
puzzles, and has a terminal mode, a browser UI and screenshot import.
It has no dependencies.

## Quickstart

You need [Rust](https://rustup.rs) installed.

```sh
cargo build --release

# Solve a puzzle (81 characters, 0 or . = empty)
./target/release/sudoku-solver 530070000600195000098000060800060003400803001700020006060000280000419005000080079

# Open the browser UI at [http://127.0.0.1:8173](http://127.0.0.1:8173)
./target/release/sudoku-solver --ui
```

## Solving engines

The solver comes with three engines. You don't need to download anything
to use them:

| Engine  | Flag      | What it does                                         |
|---------|-----------|------------------------------------------------------|
| `mrv`   | (default) | Fills in the obvious cells first, then guesses the cell with the fewest options |
| `dlx`   | `--dlx`   | Knuth's dancing links (Algorithm X)                  |
| `naive` | `--naive` | Plain backtracking, which is slow on hard puzzles    |

To run every engine on the same puzzle and compare their times:

```sh
./target/release/sudoku-solver --bench <PUZZLE>
```

## CLI commands

| Command                       | Description                                    |
|-------------------------------|------------------------------------------------|
| `sudoku-solver <PUZZLE>`      | Solve one puzzle                               |
| `-f <FILE>`                   | Solve every puzzle in a file, one per line     |
| `--dlx` / `--naive`           | Pick a different engine                        |
| `--bench <PUZZLE>`            | Compare all engines                            |
| `-w <PUZZLE>`                 | Watch the solve in the terminal                |
| `--ui`                        | Open the browser UI at http://127.0.0.1:8173   |
| `--transcribe <IMG.png>`      | Read a puzzle from an image                    |
| `--samurai <PUZZLE>`          | Treat the input as a Samurai puzzle            |
| `--help`                      | Show all options                               |

## Tests

```sh
cargo test
```

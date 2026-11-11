//! Sudoku grid representation: parsing, validation and pretty-printing.
//!
//! A [`Grid`] is an 81-cell array in row-major order. Empty cells hold `0`.

use std::fmt;

/// Full candidate bitmask: bit `i` set => digit `i + 1` is allowed.
pub const FULL: u16 = 0b111_111_111;

/// Unit index mapping: rows are units 0..9, columns 9..18, boxes 18..27.
/// `unit_cell(unit, i)` is the cell index (0..81) of the `i`-th cell of `unit`.
pub fn unit_cell(unit: usize, i: usize) -> usize {
    match unit {
        0..=8 => unit * 9 + i, // row `unit`, i-th column
        9..=17 => {
            let col = unit - 9;
            i * 9 + col // column, i-th row
        }
        _ => {
            let b = unit - 18;
            let r = (b / 3) * 3 + i / 3;
            let c = (b % 3) * 3 + i % 3;
            r * 9 + c
        }
    }
}

/// Box index (0..9) of the cell at row `r`, column `c`.
pub fn box_of(r: usize, c: usize) -> usize {
    (r / 3) * 3 + c / 3
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Grid([u8; 81]);

#[derive(Debug)]
pub enum GridError {
    WrongLength { got: usize },
    InvalidChar { ch: char },
    Duplicate { digit: u8, unit: &'static str, pos: String },
}

impl fmt::Display for GridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GridError::WrongLength { got } => {
                write!(f, "expected 81 cells after removing separators, got {got}")
            }
            GridError::InvalidChar { ch } => {
                write!(f, "invalid character {ch:?} (use 1-9, and '0' or '.' for empty)")
            }
            GridError::Duplicate { digit, unit, pos } => {
                write!(f, "duplicate clue {digit} in {unit} {pos}")
            }
        }
    }
}

impl std::error::Error for GridError {}

impl Grid {
    /// An all-empty grid. (Kept for tests and future tools like generators.)
    #[allow(dead_code)]
    pub fn empty() -> Grid {
        Grid([0; 81])
    }

    /// Build a grid from raw cells (0 = empty). No validation is performed;
    /// only use with already-validated data (e.g. solver output).
    pub fn from_cells(cells: [u8; 81]) -> Grid {
        Grid(cells)
    }

    pub fn cells(&self) -> &[u8; 81] {
        &self.0
    }

    /// The digit in the cell at row `r`, column `c` (0 = empty).
    #[allow(dead_code)]
    pub fn get(&self, r: usize, c: usize) -> u8 {
        self.0[r * 9 + c]
    }

    /// Parse a puzzle. Accepts the 81 characters of a puzzle with spaces,
    /// tabs, newlines and the box-drawing separators `-`, `+`, `|` ignored,
    /// so both compact strings and drawn grids are valid input.
    /// Digits 1-9 are clues; `0` or `.` is an empty cell.
    pub fn parse(s: &str) -> Result<Grid, GridError> {
        let trimmed: String = s
            .chars()
            .filter(|c| !matches!(c, ' ' | '\t' | '\r' | '\n' | '-' | '+' | '|'))
            .collect();

        let mut cells = [0u8; 81];
        for (i, ch) in trimmed.chars().enumerate() {
            if i >= 81 {
                return Err(GridError::WrongLength { got: trimmed.len() });
            }
            cells[i] = match ch {
                '0' | '.' => 0,
                '1'..='9' => ch as u8 - b'0',
                _ => return Err(GridError::InvalidChar { ch }),
            };
        }
        if trimmed.chars().count() != 81 {
            return Err(GridError::WrongLength {
                got: trimmed.chars().count(),
            });
        }

        let grid = Grid(cells);
        grid.validate()?;
        Ok(grid)
    }

    /// Check the given clues for duplicates within each unit.
    fn validate(&self) -> Result<(), GridError> {
        for unit in 0..27 {
            let (unit_name, unit_num) = match unit {
                0..=8 => ("row", unit as u8 + 1),
                9..=17 => ("column", unit as u8 - 8),
                _ => ("box", unit as u8 - 17),
            };
            let mut seen = 0u16;
            for i in 0..9 {
                let d = self.0[unit_cell(unit, i)];
                if d == 0 {
                    continue;
                }
                let bit = 1u16 << (d - 1);
                if seen & bit != 0 {
                    return Err(GridError::Duplicate {
                        digit: d,
                        unit: unit_name,
                        pos: unit_num.to_string(),
                    });
                }
                seen |= bit;
            }
        }
        Ok(())
    }

    /// True if the grid is completely filled and every unit contains 1..=9.
    /// (Used by tests to check solver output; exposed for future features.)
    #[allow(dead_code)]
    pub fn is_complete_and_valid(&self) -> bool {
        for unit in 0..27 {
            let mut seen = 0u16;
            for i in 0..9 {
                let d = self.0[unit_cell(unit, i)];
                if !(1..=9).contains(&d) {
                    return false;
                }
                seen |= 1u16 << (d - 1);
            }
            if seen != FULL {
                return false;
            }
        }
        true
    }

    /// One-line 81-character form (`.` for empty).
    pub fn to_compact(&self) -> String {
        self.0
            .iter()
            .map(|&d| if d == 0 { '.' } else { char::from_digit(d as u32, 10).unwrap() })
            .collect()
    }
}

impl fmt::Display for Grid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for r in 0..9 {
            if r == 3 || r == 6 {
                writeln!(f, "------+-------+------")?;
            }
            for c in 0..9 {
                if c == 3 || c == 6 {
                    write!(f, "| ")?;
                }
                let d = self.0[r * 9 + c];
                let ch = if d == 0 { '.' } else { char::from_digit(d as u32, 10).unwrap() };
                write!(f, "{ch} ")?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIKIPEDIA: &str = concat!(
        "530070000", "600195000", "098000060",
        "800060003", "400803001", "700020006",
        "060000280", "000419005", "000080079",
    );

    #[test]
    fn parses_compact() {
        let g = Grid::parse(WIKIPEDIA).expect("parses");
        assert_eq!(g.get(0, 0), 5);
        assert_eq!(g.get(8, 8), 9);
        let dotted: String = WIKIPEDIA
            .chars()
            .map(|c| if c == '0' { '.' } else { c })
            .collect();
        assert_eq!(g.to_compact(), dotted);
    }

    #[test]
    fn parses_drawn_format() {
        let drawn = "\
5 3 . | . 7 . | . . .
6 . . | 1 9 5 | . . .
. 9 8 | . . . | . 6 .
------+-------+------
8 . . | . 6 . | . . 3
4 . . | 8 . 3 | . . 1
7 . . | . 2 . | . . 6
------+-------+------
. 6 . | . . . | 2 8 .
. . . | 4 1 9 | . . 5
. . . | . 8 . | . 7 9";
        let from_drawn = Grid::parse(drawn).expect("parses drawn");
        let from_compact = Grid::parse(WIKIPEDIA).expect("parses compact");
        assert_eq!(from_drawn.cells(), from_compact.cells());
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(matches!(Grid::parse("1234"), Err(GridError::WrongLength { got: 4 })));
    }

    #[test]
    fn rejects_bad_char() {
        assert!(matches!(
            Grid::parse(".x".repeat(40).as_str()),
            Err(GridError::InvalidChar { ch: 'x' })
        ));
    }

    #[test]
    fn rejects_duplicate_in_row() {
        let mut dup = WIKIPEDIA.to_string();
        // replace char 1 (row 1 col 2, a '3') with '5' -> two 5s in row 1
        let mut chars: Vec<char> = dup.chars().collect();
        chars[1] = '5';
        dup = chars.into_iter().collect();
        assert!(matches!(
            Grid::parse(&dup),
            Err(GridError::Duplicate { digit: 5, unit: "row", .. })
        ));
    }

    #[test]
    fn rejects_duplicate_in_column() {
        let mut chars: Vec<char> = WIKIPEDIA.chars().collect();
        // cell (5,1) = 4 -> 5: duplicate 5 with (1,1) in column 1, with no
        // row or box duplicate (so the column check is what fires).
        chars[36] = '5';
        let s: String = chars.into_iter().collect();
        assert!(matches!(
            Grid::parse(&s),
            Err(GridError::Duplicate { digit: 5, unit: "column", .. })
        ));
    }

    #[test]
    fn duplicate_check_covers_boxes() {
        let mut chars: Vec<char> = WIKIPEDIA.chars().collect();
        // (2,1) = 6 -> 3: duplicate 3 with (1,2) in the top-left box,
        // different row and column (so only the box check fires).
        chars[9] = '3';
        let s: String = chars.into_iter().collect();
        assert!(matches!(
            Grid::parse(&s),
            Err(GridError::Duplicate { digit: 3, unit: "box", .. })
        ));
    }
}

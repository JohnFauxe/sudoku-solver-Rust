//! Puzzle representation for any shape (classic 9x9 or Samurai).
//!
//! A `Grid` binds a `Topology` with cell values (0 = empty). `parse()`
//! auto-detects the shape from the text: an 81-character string or drawn
//! 9x9 grid is Classic; a 441-character single line or 21 lines of 21
//! characters is Samurai (non-cell holes are `-`, ` `, `+` or `|`).

use std::fmt;

pub use crate::topo::Shape;
use crate::topo::Topology;

pub const FULL: u16 = 0b111_111_111; // bit i set => digit i+1 is allowed

#[derive(Clone)]
pub struct Grid {
    pub top: Topology,
    cells: Vec<u8>,
}

#[derive(Debug)]
pub enum GridError {
    WrongLength { shape: Shape, got: usize },
    InvalidChar { ch: char },
    Duplicate { digit: u8, unit: String },
}

impl fmt::Display for GridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GridError::WrongLength { shape, got } => match shape {
                Shape::Classic => write!(
                    f,
                    "expected 81 cells after removing separators, got {got}"
                ),
                Shape::Samurai => write!(
                    f,
                    "expected 21 lines of 21 characters (or a 441-char line), got {got} lines"
                ),
            },
            GridError::InvalidChar { ch } => {
                write!(f, "invalid character {ch:?} (use 1-9, and '0' or '.' for empty)")
            }
            GridError::Duplicate { digit, unit } => {
                write!(f, "duplicate clue {digit} in {unit}")
            }
        }
    }
}

impl std::error::Error for GridError {}

fn digit_char(d: u8) -> char {
    if d == 0 {
        '.'
    } else {
        char::from_digit(d as u32, 10).unwrap()
    }
}

impl Grid {
    /// Grid on `top` with all cells empty. (Used by tests and tools.)
    #[allow(dead_code)]
    pub fn new(top: Topology) -> Grid {
        let cells = vec![0; top.cells];
        Grid { top, cells }
    }

    /// An empty classic grid.
    #[allow(dead_code)]
    pub fn empty() -> Grid {
        Grid::new(Topology::new(Shape::Classic))
    }

    /// The puzzle shape. (Kept for API completeness; use `top.shape`.)
    #[allow(dead_code)]
    pub fn shape(&self) -> Shape {
        self.top.shape
    }

    /// Build from raw cell values (0 = empty). No validation is performed;
    /// only use with already-validated data (e.g. solver output).
    pub fn from_cells(top: Topology, cells: Vec<u8>) -> Grid {
        Grid { top, cells }
    }

    pub fn cells(&self) -> &[u8] {
        &self.cells
    }

    /// The digit at display position (r, c) (0 = empty or non-cell).
    #[allow(dead_code)]
    pub fn get(&self, r: usize, c: usize) -> u8 {
        let idx = self.top.index[r][c];
        if idx == usize::MAX {
            0
        } else {
            self.cells[idx]
        }
    }

    /// Parse a puzzle, auto-detecting Classic vs Samurai from the text.
    pub fn parse(s: &str) -> Result<Grid, GridError> {
        let shape = detect_shape(s);
        Grid::parse_shape(s, Some(shape))
    }

    /// Parse with a forced shape (`Some(shape)`), or auto-detect (`None`).
    pub fn parse_shape(s: &str, force: Option<Shape>) -> Result<Grid, GridError> {
        match force.unwrap_or_else(|| detect_shape(s)) {
            Shape::Classic => parse_classic(s),
            Shape::Samurai => parse_samurai(s),
        }
    }

    /// Check the given clues for duplicates within each unit.
    fn validate(&self) -> Result<(), GridError> {
        for unit in 0..self.top.units {
            let mut seen = 0u16;
            for &cell in &self.top.unit_cells[unit] {
                let d = self.cells[cell];
                if d == 0 {
                    continue;
                }
                let bit = 1u16 << (d - 1);
                if seen & bit != 0 {
                    return Err(GridError::Duplicate {
                        digit: d,
                        unit: self.top.unit_name(unit),
                    });
                }
                seen |= bit;
            }
        }
        Ok(())
    }

    /// True if the grid is completely filled and every unit contains 1..=9.
    pub fn is_complete_and_valid(&self) -> bool {
        for &d in &self.cells {
            if d == 0 {
                return false;
            }
        }
        for cells in &self.top.unit_cells {
            let mut seen = 0u16;
            for &cell in cells {
                seen |= 1u16 << (self.cells[cell] - 1);
            }
            if seen != FULL {
                return false;
            }
        }
        true
    }

    /// Compact one-line form in cell order (`.` for empty). Classic: 81
    /// characters; Samurai: 369.
    pub fn to_compact(&self) -> String {
        self.cells.iter().map(|&d| digit_char(d)).collect()
    }

    /// Text form: Classic as an 81-char line; Samurai as 21 lines of 21
    /// characters with `-` marking non-cells. (Used by tests and round-trips.)
    #[allow(dead_code)]
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        if self.top.shape == Shape::Classic {
            return self.to_compact();
        }
        for r in 0..self.top.h {
            for c in 0..self.top.w {
                let idx = self.top.index[r][c];
                s.push(if idx == usize::MAX {
                    '-'
                } else {
                    digit_char(self.cells[idx])
                });
            }
            s.push('\n');
        }
        s
    }
}

/// Classic: accept the 81 significant characters of a puzzle with
/// whitespace and the separators `-`, `+`, `|` ignored.
fn parse_classic(s: &str) -> Result<Grid, GridError> {
    let trimmed: Vec<char> = s
        .chars()
        .filter(|c| !matches!(c, ' ' | '\t' | '\r' | '\n' | '-' | '+' | '|'))
        .collect();
    if trimmed.len() != 81 {
        return Err(GridError::WrongLength {
            shape: Shape::Classic,
            got: trimmed.len(),
        });
    }
    let mut cells = vec![0u8; 81];
    for (i, &ch) in trimmed.iter().enumerate() {
        cells[i] = match ch {
            '0' | '.' => 0,
            '1'..='9' => ch as u8 - b'0',
            _ => return Err(GridError::InvalidChar { ch }),
        };
    }
    let grid = Grid::from_cells(Topology::new(Shape::Classic), cells);
    grid.validate()?;
    Ok(grid)
}

/// Samurai: 21 display rows of 21 characters (or one 441-char line).
/// At cell positions digits/`0`/`.` are read; at hole positions any
/// non-digit character is accepted. Trailing whitespace per line is
/// trimmed, and short lines are padded only where the missing
/// characters are all non-cells.
fn parse_samurai(s: &str) -> Result<Grid, GridError> {
    // Gather exactly 21 display rows, each 21 characters.
    let mut rows: Vec<Vec<char>> = Vec::new();
    if !s.contains('\n') {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() != 441 {
            return Err(GridError::WrongLength {
                shape: Shape::Samurai,
                got: chars.len(),
            });
        }
        for r in 0..21 {
            rows.push(chars[r * 21..(r + 1) * 21].to_vec());
        }
    } else {
        for line in s.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                continue;
            }
            let chars: Vec<char> = line.chars().collect();
            if chars.len() > 21 {
                return Err(GridError::WrongLength {
                    shape: Shape::Samurai,
                    got: rows.len() + chars.len(),
                });
            }
            rows.push(chars);
            if rows.len() == 21 {
                break;
            }
        }
        if rows.len() != 21 {
            return Err(GridError::WrongLength {
                shape: Shape::Samurai,
                got: rows.len(),
            });
        }
    }

    let top = Topology::new(Shape::Samurai);
    let mut cells = vec![0u8; top.cells];
    for r in 0..21 {
        // Pad short lines, but only with non-cells.
        while rows[r].len() < 21 {
            let c = rows[r].len();
            if top.index[r][c] == usize::MAX {
                rows[r].push('-');
            } else {
                return Err(GridError::WrongLength {
                    shape: Shape::Samurai,
                    got: rows[r].len(),
                });
            }
        }
        for c in 0..21 {
            let idx = top.index[r][c];
            if idx == usize::MAX {
                continue; // hole: ignore whatever is there
            }
            let ch = rows[r][c];
            cells[idx] = match ch {
                '0' | '.' => 0,
                '1'..='9' => ch as u8 - b'0',
                _ => return Err(GridError::InvalidChar { ch }),
            };
        }
    }
    let grid = Grid::from_cells(top, cells);
    grid.validate()?;
    Ok(grid)
}

/// Guess the shape from the text layout (see module docs).
fn detect_shape(s: &str) -> Shape {
    if !s.contains('\n') {
        return if s.chars().count() == 441 {
            Shape::Samurai
        } else {
            Shape::Classic
        };
    }
    let nonblank = s.lines().filter(|l| !l.trim_end().is_empty()).count();
    if nonblank == 21 {
        Shape::Samurai
    } else {
        Shape::Classic
    }
}

/// Format a digit for display ('.' for empty).
pub fn cell_char(d: u8) -> char {
    digit_char(d)
}

impl fmt::Display for Grid {
    /// Render with box separators at each host grid's internal 3/6
    /// boundaries; non-cells are blanks.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (w, h) = (self.top.w, self.top.h);
        for r in 0..h {
            // Separator line above this row?
            let sep_here = (0..w)
                .any(|c| {
                    let idx = self.top.index[r][c];
                    idx != usize::MAX && self.top.top_border[idx]
                });
            if sep_here {
                for c in 0..w {
                    let above = r > 0 && self.top.index[r - 1][c] != usize::MAX;
                    let below = self.top.index[r][c] != usize::MAX;
                    if above || below {
                        let cross =
                            (above && self.top.left_border[self.top.index[r - 1][c]])
                                || (below && self.top.left_border[self.top.index[r][c]]);
                        write!(f, "{}", if cross { '+' } else { '-' })?;
                    } else {
                        write!(f, " ")?;
                    }
                }
                writeln!(f)?;
            }
            for c in 0..w {
                let idx = self.top.index[r][c];
                if idx == usize::MAX {
                    write!(f, " ")?;
                } else {
                    if self.top.left_border[idx] {
                        write!(f, "|")?;
                    }
                    write!(f, "{}", cell_char(self.cells[idx]))?;
                }
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

    /// Canonical empty Samurai text: 21 lines of 21 chars, `.` = empty
    /// cell, `-` = non-cell.
    fn samurai_empty_text() -> String {
        Grid::new(Topology::new(Shape::Samurai)).to_text()
    }

    #[test]
    fn topology_sizes() {
        let c = Topology::new(Shape::Classic);
        assert_eq!((c.cells, c.units, c.w, c.h), (81, 27, 9, 9));
        let s = Topology::new(Shape::Samurai);
        assert_eq!((s.cells, s.units, s.w, s.h), (369, 135, 21, 21));
        // Shared cell (6,6): TL (grid 0) + center (grid 2) => bitmask 0b101.
        let shared = s.index[6][6];
        assert_eq!(s.cell_units[shared].len(), 6);
        assert_eq!(s.grids[shared], 0b101); // TL + center
        // A center-only cell.
        let mid = s.index[10][10];
        assert_eq!(s.cell_units[mid].len(), 3);
        // Every unit has exactly 9 cells.
        for uc in &s.unit_cells {
            assert_eq!(uc.len(), 9);
        }
    }

    #[test]
    fn parses_compact_classic() {
        let g = Grid::parse(WIKIPEDIA).expect("parses");
        assert_eq!(g.shape(), Shape::Classic);
        assert_eq!(g.get(0, 0), 5);
        assert_eq!(g.get(8, 8), 9);
        assert_eq!(g.to_compact(), WIKIPEDIA.replace('0', "."));
    }

    #[test]
    fn parses_drawn_classic() {
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
        let a = Grid::parse(drawn).unwrap();
        let b = Grid::parse(WIKIPEDIA).unwrap();
        assert_eq!(a.cells(), b.cells());
    }

    #[test]
    fn rejects_wrong_length_classic() {
        assert!(matches!(
            Grid::parse("1234"),
            Err(GridError::WrongLength { shape: Shape::Classic, got: 4 })
        ));
    }

    #[test]
    fn rejects_short_classic() {
        assert!(matches!(
            Grid::parse(".x".repeat(40).as_str()),
            Err(GridError::WrongLength { shape: Shape::Classic, got: 80 })
        ));
    }

    #[test]
    fn rejects_duplicate_in_row() {
        let mut chars: Vec<char> = WIKIPEDIA.chars().collect();
        chars[1] = '5'; // (0,1): two 5s in row 1
        let s: String = chars.into_iter().collect();
        assert!(matches!(
            Grid::parse(&s),
            Err(GridError::Duplicate { digit: 5, .. })
        ));
    }

    #[test]
    fn duplicate_check_covers_columns_and_boxes() {
        // (5,1) 4 -> 5: duplicate in column 1 only.
        let mut chars: Vec<char> = WIKIPEDIA.chars().collect();
        chars[36] = '5';
        let s: String = chars.into_iter().collect();
        assert!(matches!(
            Grid::parse(&s),
            Err(GridError::Duplicate { digit: 5, .. })
        ));
        // (2,1) 6 -> 3: duplicate 3 in the top-left box only.
        let mut chars: Vec<char> = WIKIPEDIA.chars().collect();
        chars[9] = '3';
        let s: String = chars.into_iter().collect();
        assert!(matches!(
            Grid::parse(&s),
            Err(GridError::Duplicate { digit: 3, .. })
        ));
    }

    #[test]
    fn parses_samurai_empty() {
        let g = Grid::parse(&samurai_empty_text()).expect("parses samurai");
        assert_eq!(g.shape(), Shape::Samurai);
        assert_eq!(g.cells().len(), 369);
        assert!(g.cells().iter().all(|&d| d == 0));
    }

    #[test]
    fn parses_samurai_single_line() {
        let single: String = samurai_empty_text()
            .lines()
            .flat_map(|l| l.chars())
            .collect();
        assert_eq!(single.chars().count(), 21 * 21);
        let g = Grid::parse(&single).expect("parses single-line samurai");
        assert_eq!(g.shape(), Shape::Samurai);
        assert!(g.cells().iter().all(|&d| d == 0));
    }

    #[test]
    fn samurai_round_trip() {
        let g = Grid::parse(&samurai_empty_text()).unwrap();
        // Round-trip through to_text: '-' holes, then re-parse.
        let text = g.to_text();
        let g2 = Grid::parse(&text).expect("re-parses");
        assert_eq!(g2.cells(), g.cells());
    }

    #[test]
    fn samurai_rejects_garbage_in_cells() {
        // Put an 'x' at the cell position display (0,0).
        let bad = samurai_empty_text()
            .lines()
            .enumerate()
            .map(|(r, l)| {
                if r == 0 {
                    let mut v: Vec<char> = l.chars().collect();
                    v[0] = 'x';
                    v.into_iter().collect::<String>()
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(matches!(
            Grid::parse(&bad),
            Err(GridError::InvalidChar { ch: 'x' })
        ));
    }

    #[test]
    fn samurai_duplicate_in_shared_box_is_rejected() {
        // The shared box (display 6..8, 6..8) belongs to two grids; place
        // two 5s inside it, one seen through each grid's row.
        let mut text = samurai_empty_text();
        let mut lines: Vec<Vec<char>> =
            text.lines().map(|l| l.chars().collect()).collect();
        assert_eq!(lines[6][6], '.');
        assert_eq!(lines[7][7], '.');
        lines[6][6] = '5';
        lines[7][7] = '5';
        text = lines.iter().map(|l| l.iter().collect::<String>()).collect::<String>();
        assert!(matches!(
            Grid::parse(&text),
            Err(GridError::Duplicate { digit: 5, .. })
        ));
    }
}

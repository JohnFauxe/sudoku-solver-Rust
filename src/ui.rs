//! `--ui` mode: a local web server (std-only) that opens the browser UI.
//!
//!  - `GET /`          -> the embedded page (`ui_page.html`)
//!  - `POST /api/solve` -> JSON { puzzle, maxSolutions, allowSearch } and the
//!    full step/event log, which the page animates with playback controls.
//!
//! Listens on 127.0.0.1 only. No dependencies: hand-rolled HTTP + JSON.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Instant;

use crate::grid::Grid;
use crate::solver::{
    solve_with, Algorithm, Placer, SolveConfig, SolveOutcome, StepInfo, StepLogger,
};

const PAGE: &str = include_str!("ui_page.html");
/// Refuse responses bigger than this (about 400 000 events).
const MAX_EVENTS: usize = 400_000;

struct EventCollector {
    events: Vec<String>, // {"s":"<cells>","i":<idx|null>,"d":<digit|null>,"p":"n|h|g|d|b|u|null"}
    truncated: bool,
}

impl StepLogger for EventCollector {
    fn step(&mut self, cells: &[u8], info: Option<StepInfo>) {
        if self.events.len() >= MAX_EVENTS {
            self.truncated = true;
            return;
        }
        let s: String = cells
            .iter()
            .map(|&d| if d == 0 { '.' } else { char::from_digit(d as u32, 10).unwrap() })
            .collect();
        let (i, d, p) = match info {
            Some(info) => (
                info.idx.to_string(),
                info.digit.to_string(),
                match info.placer {
                    Placer::Naked => "n",
                    Placer::Hidden => "h",
                    Placer::Guess => "g",
                    Placer::Dlx => "d",
                    Placer::Naive => "b",
                    Placer::Undo => "u",
                },
            ),
            None => ("null".into(), "null".into(), "null"),
        };
        self.events.push(format!(r#"{{"s":"{s}","i":{i},"d":{d},"p":"{p}"}}"#));
    }
}

pub fn serve(port: u16, open_browser: bool) -> ! {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: cannot listen on 127.0.0.1:{port}: {e}");
            eprintln!("       (is another instance running? try --port <N>)");
            std::process::exit(1);
        }
    };
    let addr = listener.local_addr().expect("addr");
    let url = format!("http://{addr}");
    println!("sudoku UI listening on {url}  (Ctrl-C to stop)");
    println!("  opening your browser… (use --no-open to skip)");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    if open_browser {
        let opened = ["xdg-open", "open", "x-www-browser"]
            .iter()
            .any(|cmd| std::process::Command::new(cmd).arg(&url).spawn().is_ok());
        if !opened {
            println!("  could not auto-open a browser; visit the URL above.");
        }
    }

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                thread::spawn(move || handle_conn(s));
            }
            Err(e) => eprintln!("connection error: {e}"),
        }
    }
    unreachable!("listener.incoming() never ends")
}

fn handle_conn(mut stream: TcpStream) {
    if let Ok((method, path, body)) = read_request(&mut stream) {
        let (code, ctype, payload) = route(&method, &path, body);
        write_response(&mut stream, code, ctype, &payload);
    }
}

fn route(method: &str, raw_path: &str, body: Vec<u8>) -> (&'static str, &'static str, String) {
    // Strip any query string (?puzzle=…) — the page reads it itself.
    let path = raw_path.split('?').next().unwrap_or("/");
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => {
            ("200 OK", "text/html; charset=utf-8", PAGE.to_string())
        }
        ("POST", "/api/solve") => match String::from_utf8(body) {
            Ok(text) => solve_api(&text),
            Err(_) => error_response("request body is not UTF-8"),
        },
        ("GET", _) => ("404 Not Found", "text/plain", "not found\n".into()),
        _ => ("405 Method Not Allowed", "text/plain", "bad method\n".into()),
    }
}

fn error_response(msg: &str) -> (&'static str, &'static str, String) {
    ("400 Bad Request", "application/json", format!(r#"{{"ok":false,"error":"{}"}}"#, json_escape(msg)))
}

fn solve_api(body: &str) -> (&'static str, &'static str, String) {
    let Some(puzzle) = json_str_field(body, "puzzle") else {
        return error_response("missing \"puzzle\" field");
    };
    let max_solutions = json_num_field(body, "maxSolutions").unwrap_or(2);
    let allow_search = json_bool_field(body, "allowSearch", true);
    let alg_name = json_str_field(body, "algorithm").unwrap_or_default();
    let algorithm = match alg_name.as_str() {
        "dlx" => Algorithm::DancingLinks,
        "naive" => Algorithm::NaiveBacktracking,
        _ => Algorithm::Propagation,
    };
    let cfg = SolveConfig {
        max_solutions: max_solutions.clamp(1, 256) as usize,
        allow_search,
        algorithm,
    };

    let Ok(grid) = Grid::parse(&puzzle) else {
        return error_response("bad puzzle: use 81 chars (classic), 21 lines of 21 or a 441-char line (samurai); no duplicate clues");
    };
    let start = Instant::now();
    let mut collector = EventCollector { events: Vec::new(), truncated: false };
    let outcome = solve_with(&grid, cfg, Some(&mut collector));
    let elapsed = start.elapsed().as_millis();
    let clue_str: String = grid
        .cells()
        .iter()
        .map(|&d| if d == 0 { '.' } else { char::from_digit(d as u32, 10).unwrap() })
        .collect();

    // Layout metadata so the page can draw any topology: display size and,
    // per cell (in solver cell order), its display position and borders.
    let layout_json = layout_to_json(&grid);

    // An initial "start" event so the replay always begins at the imported
    // puzzle; then all solver placements in order.
    let mut events = String::with_capacity(64 * collector.events.len() + 256);
    events.push_str("{\"s\":\"");
    events.push_str(&clue_str);
    events.push_str("\",\"i\":null,\"d\":null,\"p\":null}");
    for e in &collector.events {
        events.push(',');
        events.push_str(e);
    }

    let outcome_json = match &outcome {
        SolveOutcome::Unique { .. } => "unique",
        SolveOutcome::NoSolution { .. } => "none",
        SolveOutcome::Multiple { .. } => "multiple",
    };
    // For unique outcomes the search kept branching to prove uniqueness after
    // the first board was complete; the page trims there, but flag it anyway.
    let incomplete = matches!(&outcome, SolveOutcome::NoSolution { .. }) && !allow_search;
    let solutions_json = match &outcome {
        SolveOutcome::Unique { solution, .. } => {
            format!(r#"["{}"]"#, json_escape(&solution.to_compact()))
        }
        SolveOutcome::Multiple { solutions, .. } => {
            let v: Vec<String> = solutions
                .iter()
                .map(|s| format!(r#""{}""#, json_escape(&s.to_compact())))
                .collect();
            format!("[{}]", v.join(","))
        }
        SolveOutcome::NoSolution { .. } => "[]".to_string(),
    };

    let stats_tuple = match &outcome {
        SolveOutcome::Unique { stats, .. } | SolveOutcome::Multiple { stats, .. } => {
            (stats.naked_singles, stats.hidden_singles, stats.guesses, stats.backtracks)
        }
        SolveOutcome::NoSolution { stats } => {
            (stats.naked_singles, stats.hidden_singles, stats.guesses, stats.backtracks)
        }
    };

    let json = format!(
        concat!(
            r#"{{"ok":true,"outcome":"{}","incomplete":{},"puzzle":"{}","layout":{},"#,
            r#""stats":{{"naked":{},"hidden":{},"guesses":{},"dead":{}}},"#,
            r#""solutions":{},"events":[{}],"truncated":{},"elapsedMs":{}}}"#
        ),
        outcome_json,
        incomplete,
        json_escape(&clue_str),
        layout_json,
        stats_tuple.0,
        stats_tuple.1,
        stats_tuple.2,
        stats_tuple.3,
        solutions_json,
        events,
        collector.truncated,
        elapsed,
    );
    ("200 OK", "application/json", json)
}

/// Serialize a topology for the page: display size plus one entry per cell
/// (solver cell order) with its display position and box borders.
fn layout_to_json(grid: &Grid) -> String {
    let top = &grid.top;
    let mut cells = String::with_capacity(top.cells * 48);
    for (idx, &(r, c)) in top.positions.iter().enumerate() {
        if idx > 0 {
            cells.push(',');
        }
        cells.push_str(&format!(
            "{{\"r\":{},\"c\":{},\"bl\":{},\"bt\":{}}}",
            r,
            c,
            top.left_border[idx],
            top.top_border[idx]
        ));
    }
    format!(
        r#"{{"w":{},"h":{},"cells":[{}]}}"#,
        top.w, top.h, cells
    )
}

fn write_response(stream: &mut TcpStream, code: &str, ctype: &str, body: &str) {
    let mut resp = format!(
        "HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    resp.extend_from_slice(body.as_bytes());
    let _ = stream.write_all(&resp);
}

/// Read one HTTP request (headers + body) from the connection.
fn read_request(stream: &mut TcpStream) -> std::io::Result<(String, String, Vec<u8>)> {
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_len = loop {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof"));
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 1 << 20 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "headers too large"));
        }
    };
    let headers = String::from_utf8_lossy(&buf[..header_len]).into_owned();
    let mut lines = headers.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut content_length = 0usize;
    for line in lines {
        let mut kv = line.splitn(2, ':');
        let (key, value) = (kv.next().unwrap_or(""), kv.next().unwrap_or(""));
        if key.eq_ignore_ascii_case("content-length") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    while buf.len() < header_len + content_length {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let body_len = buf.len().min(header_len + content_length);
    Ok((method, path, buf[header_len..body_len].to_vec()))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

// --- minimal JSON decoding (bodies come only from our own page) ---

fn json_str_field(body: &str, key: &str) -> Option<String> {
    let start = body.find(&format!(r#""{key}""#))? + key.len() + 2;
    let rest = &body[start.min(body.len())..];
    let colon = rest.find(':')?;
    let after = &body[(start + colon).min(body.len())..];
    let q = after.find('"')?;
    let mut out = String::new();
    let mut chars = after[q + 1..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => out.push(other),
                None => break,
            },
            _ => out.push(c),
        }
    }
    Some(out)
}

fn json_num_field(body: &str, key: &str) -> Option<i64> {
    let after = value_start(body, key)?;
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    digits.parse().ok()
}

fn json_bool_field(body: &str, key: &str, default: bool) -> bool {
    let Some(after) = value_start(body, key) else { return default };
    if after.starts_with("false") { false } else { default }
}

/// Position right after the `:` of `"key":`.
fn value_start<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let start = body.find(&format!(r#""{key}""#))? + key.len() + 2;
    let rest = &body[start.min(body.len())..];
    let colon = rest.find(':')?;
    rest[colon + 1..].split_whitespace().next().map(|_| &rest[colon + 1..])
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

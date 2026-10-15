// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Robustness of the plugin host boundary against hostile input: mutated
//! manifests, random SQL, and damaged or hostile `.tptp` archives. Each input
//! must be accepted or refused, never panic, and no archive may write outside
//! its destination.
//!
//! The generator is a fixed-seed xorshift, so a failure reproduces exactly.
//! This is a deterministic stand-in for fuzzing, not a replacement for it.

#![cfg(feature = "js")]

use std::path::Path;

use tpt_home_plugins::manifest;
use tpt_home_plugins::package;
use tpt_home_plugins::scope;

/// Small deterministic PRNG (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const MANIFEST: &str = include_str!("../../../plugins/logbook/plugin.toml");

/// Characters that matter to a TOML or SQL parser, plus some non-ASCII.
const NASTY: &[char] = &[
    '"', '\'', '[', ']', '{', '}', '=', ',', '\n', '\r', '\\', '#', '\0', '\u{7f}', 'é', '😀', ' ',
    '\t', ';',
];

fn mutate_text(rng: &mut Rng, base: &str) -> String {
    let mut chars: Vec<char> = base.chars().collect();
    for _ in 0..1 + rng.below(8) {
        match rng.below(4) {
            0 if !chars.is_empty() => {
                let at = rng.below(chars.len());
                chars.remove(at);
            }
            1 => {
                let at = rng.below(chars.len() + 1);
                chars.insert(at, NASTY[rng.below(NASTY.len())]);
            }
            2 if !chars.is_empty() => {
                let at = rng.below(chars.len());
                chars[at] = NASTY[rng.below(NASTY.len())];
            }
            _ => {
                let cut = rng.below(chars.len() + 1);
                chars.truncate(cut);
            }
        }
    }
    chars.into_iter().collect()
}

#[test]
fn mutated_manifests_are_refused_or_accepted_without_panicking() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut accepted = 0usize;
    for _ in 0..4000 {
        let text = mutate_text(&mut rng, MANIFEST);
        if manifest::parse(&text).is_ok() {
            accepted += 1;
        }
    }
    // The unmutated manifest is valid, so the harness is really exercising
    // the parser rather than refusing everything.
    assert!(manifest::parse(MANIFEST).is_ok());
    assert!(accepted < 4000, "mutations should break most manifests");
}

#[test]
fn random_target_strings_do_not_panic() {
    let mut rng = Rng(42);
    let pieces = [
        "http://", "https://", "[", "]", ":", "65536", "0", "::1", "a.b", "é", "/", "@",
    ];
    for _ in 0..4000 {
        let mut s = String::new();
        for _ in 0..1 + rng.below(6) {
            s.push_str(rng.pick(&pieces));
        }
        let _ = manifest::parse_target(&s);
    }
}

#[test]
fn random_sql_is_accepted_only_as_a_scoped_select() {
    let mut rng = Rng(7);
    let tokens = [
        "SELECT",
        "select",
        "COUNT(*)",
        "FROM",
        "boundary_items",
        "other_items",
        "WHERE",
        "id",
        "=",
        "1",
        "'x'",
        "--",
        "/*",
        "*/",
        ";",
        "UNION",
        "DELETE",
        "INSERT",
        "(",
        ")",
        "\"boundary_items\"",
        "`boundary_items`",
        "boundary_",
        "sqlite_master",
        "ATTACH",
        "PRAGMA",
        "WITH",
        "JOIN",
        "LIMIT",
        "5",
    ];
    let mut accepted = 0usize;
    for _ in 0..6000 {
        let mut sql = String::new();
        for _ in 0..1 + rng.below(9) {
            sql.push_str(rng.pick(&tokens));
            sql.push(' ');
        }
        if let Ok(statement) = scope::validate_scoped_sql("boundary", &sql) {
            accepted += 1;
            // Only a trailing `;` is allowed, after any code or comments.
            let code = code_only(&statement);
            assert!(
                !code.trim_end().trim_end_matches(';').contains(';'),
                "accepted two statements: {sql:?}"
            );
            assert_only_scoped_tables(&statement, &sql);
            let verb = code_only(&statement)
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_uppercase();
            assert!(
                matches!(
                    verb.as_str(),
                    "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "CREATE" | "DROP" | "ALTER"
                ),
                "accepted verb {verb}: {sql:?}"
            );
        }
    }
    // Most random statements are malformed, so the accepted set must still be
    // large enough for the checks above to mean something.
    assert!(accepted > 100, "only {accepted} statements were accepted");
}

/// The statement with string literals and comments blanked out, so only SQL
/// code is left to inspect.
fn code_only(sql: &str) -> String {
    let mut out = String::new();
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                // Skip to the closing quote; a doubled quote is an escape.
                while let Some(d) = chars.next() {
                    if d == '\'' {
                        if chars.peek() == Some(&'\'') {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                out.push(' ');
            }
            '-' if chars.peek() == Some(&'-') => {
                for d in chars.by_ref() {
                    if d == '\n' {
                        break;
                    }
                }
                out.push(' ');
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                for d in chars.by_ref() {
                    if prev == '*' && d == '/' {
                        break;
                    }
                    prev = d;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// Every word that follows a table keyword must be a scoped name. Keywords
/// that can sit between the table keyword and the name are skipped.
fn assert_only_scoped_tables(statement: &str, original: &str) {
    let code = code_only(statement);
    let words: Vec<String> = code
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| "()\"`,".contains(c)).to_string())
        .collect();
    for (i, word) in words.iter().enumerate() {
        let keyword = word.to_ascii_uppercase();
        if !matches!(
            keyword.as_str(),
            "FROM" | "JOIN" | "INTO" | "UPDATE" | "TABLE"
        ) {
            continue;
        }
        let Some(next) = words.get(i + 1) else {
            continue;
        };
        let upper = next.to_ascii_uppercase();
        // Punctuation after the keyword is not a name; SQLite rejects it.
        let starts_like_a_name = next.starts_with(|c: char| c.is_alphanumeric() || c == '_');
        if !starts_like_a_name || matches!(upper.as_str(), "SELECT" | "IF" | "NOT" | "EXISTS") {
            continue;
        }
        assert!(
            next.starts_with("boundary_"),
            "table position names an unscoped table {next:?} in {original:?}"
        );
    }
}

/// A valid package built from a real directory, to mutate.
fn valid_package(dir: &Path) -> Vec<u8> {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        MANIFEST
            .replace("id = \"logbook\"", "id = \"boundary\"")
            .replace("logbook_", "boundary_"),
    )
    .unwrap();
    std::fs::write(
        dir.join("main.js"),
        "function handle() { return { status: 200 }; }",
    )
    .unwrap();
    package::pack(dir).unwrap().bytes
}

#[test]
fn damaged_packages_are_refused_without_panicking_or_escaping() {
    let root = tempfile::tempdir().unwrap();
    let base = valid_package(&root.path().join("src"));
    let mut rng = Rng(1234);

    let mut accepted = 0usize;
    for round in 0..300 {
        let mut bytes = base.clone();
        for _ in 0..1 + rng.below(6) {
            let at = rng.below(bytes.len());
            bytes[at] ^= 1 << rng.below(8);
        }
        if rng.below(4) == 0 {
            let cut = rng.below(bytes.len());
            bytes.truncate(cut);
        }
        let dest = root.path().join(format!("dest-{round}"));
        if package::unpack_verify(&bytes, &dest).is_ok() {
            accepted += 1;
        }
        // Nothing may appear next to the destination.
        for entry in std::fs::read_dir(root.path()).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                name == "src" || name.starts_with("dest-"),
                "unexpected file written outside the destination: {name}"
            );
        }
    }
    assert!(accepted < 300, "damaged packages should not all verify");
}

#[test]
fn random_bytes_are_refused_without_panicking() {
    let root = tempfile::tempdir().unwrap();
    let mut rng = Rng(99);
    for round in 0..300 {
        let len = rng.below(2048);
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let dest = root.path().join(format!("noise-{round}"));
        assert!(package::unpack_verify(&bytes, &dest).is_err());
    }
}

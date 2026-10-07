// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! SQL scoping for plugin database access.
//!
//! Plugins get raw SQL execution, but only against tables namespaced with
//! their own id (`<id>_*`). The validator is deliberately paranoid:
//!
//! - exactly one statement (no stacked `;`),
//! - statement verbs restricted to `SELECT/INSERT/UPDATE/DELETE` and
//!   `CREATE TABLE` / `DROP TABLE` / `ALTER TABLE`,
//! - no `ATTACH`, `PRAGMA`, `VACUUM`, `REINDEX`, `ANALYZE`, `EXPLAIN`,
//! - every table-ish identifier (after `FROM`, `JOIN`, `INTO`, `UPDATE`,
//!   `TABLE`, `RENAME TO`) must match `<id>_<name>` with a strict charset,
//! - quoted table names are banned outright (the scope model is
//!   identifier-based, no exceptions),
//! - string literals and comments are opaque tokens, so keywords hidden
//!   inside them cannot smuggle syntax past the scanner.
//!
//! This is defense-in-depth on top of SQLite itself; the plugin connection
//! is a separate handle and the host re-checks at prepare time.

use tpt_home_core::ApiError;

/// Statement keywords that must never appear outside string literals.
const FORBIDDEN_KEYWORDS: [&str; 6] =
    ["ATTACH", "DETACH", "PRAGMA", "VACUUM", "REINDEX", "EXPLAIN"];

/// A minimal SQL tokenizer good enough for scoping decisions.
#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Quoted(String), // '…' string or "…" identifier — opaque
    Punct(char),
}

fn tokenize(sql: &str) -> Result<Vec<Tok>, ApiError> {
    let chars: Vec<char> = sql.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '\'' || c == '"' || c == '`' {
            let start = i + 1;
            let mut j = start;
            let mut closed = false;
            while j < chars.len() {
                if chars[j] == c {
                    if j + 1 < chars.len() && chars[j + 1] == c {
                        j += 2; // escaped quote
                        continue;
                    }
                    closed = true;
                    break;
                }
                j += 1;
            }
            if !closed {
                return Err(ApiError::bad_request("plugin SQL: unterminated quote"));
            }
            toks.push(Tok::Quoted(chars[start..j].iter().collect()));
            i = j + 1;
        } else if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            toks.push(Tok::Word(chars[start..i].iter().collect()));
        } else if c == '-' && i + 1 < chars.len() && chars[i + 1] == '-' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            let mut j = i + 2;
            let mut closed = false;
            while j + 1 < chars.len() {
                if chars[j] == '*' && chars[j + 1] == '/' {
                    closed = true;
                    break;
                }
                j += 1;
            }
            if !closed {
                return Err(ApiError::bad_request("plugin SQL: unterminated comment"));
            }
            i = j + 2;
        } else {
            toks.push(Tok::Punct(c));
            i += 1;
        }
    }
    Ok(toks)
}

/// Validate that `sql` is a single statement touching only `<plugin_id>_`
/// tables. Returns the statement with the trailing `;` trimmed.
pub fn validate_scoped_sql(plugin_id: &str, sql: &str) -> Result<String, ApiError> {
    if sql.trim().is_empty() {
        return Err(ApiError::bad_request("plugin SQL: empty statement"));
    }
    let trimmed = sql.trim();
    if trimmed.len() > 8_192 {
        return Err(ApiError::bad_request("plugin SQL: statement too long"));
    }

    let toks = tokenize(trimmed)?;

    // Single statement: any `;` must be the final token.
    let semis = toks.iter().filter(|t| **t == Tok::Punct(';')).count();
    if semis > 1 || (semis == 1 && !matches!(toks.last(), Some(Tok::Punct(';')))) {
        return Err(ApiError::bad_request(
            "plugin SQL: only one statement is allowed",
        ));
    }

    // Forbidden keywords anywhere outside string literals.
    let words: Vec<String> = toks
        .iter()
        .filter_map(|t| match t {
            Tok::Word(w) => Some(w.to_ascii_uppercase()),
            _ => None,
        })
        .collect();
    for kw in FORBIDDEN_KEYWORDS {
        if words.iter().any(|w| w == kw) {
            return Err(ApiError::bad_request(format!("plugin SQL: {kw} is not permitted")));
        }
    }

    // The statement verb must be one of the safe verbs.
    let Some(Tok::Word(verb_tok)) = toks.first() else {
        return Err(ApiError::bad_request("plugin SQL: expected a statement"));
    };
    let verb = verb_tok.to_ascii_uppercase();
    match verb.as_str() {
        "SELECT" | "INSERT" | "UPDATE" | "DELETE" => {}
        "CREATE" | "DROP" | "ALTER" => {
            // DDL is table-only: CREATE/DROP/ALTER <other> (INDEX, TRIGGER,
            // VIEW, …) would allow arbitrary-SQL side effects.
            let second = toks.get(1).and_then(|t| match t {
                Tok::Word(w) => Some(w.to_ascii_uppercase()),
                _ => None,
            });
            if second.as_deref() != Some("TABLE") {
                return Err(ApiError::bad_request(format!(
                    "plugin SQL: {verb} is only permitted as {verb} TABLE"
                )));
            }
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "plugin SQL: '{other}' statements are not permitted"
            )));
        }
    }

    // Every identifier after a table-position keyword must be scoped. The
    // flag is cleared by any punctuation, so `VALUES (…)`, sub-selects after
    // `(`, etc. cannot inherit a stale table position.
    let prefix = format!("{plugin_id}_");
    let mut table_pos = false;
    let mut alter_rename = verb == "ALTER";
    for tok in &toks {
        match tok {
            Tok::Word(w) => {
                let upper = w.to_ascii_uppercase();
                if table_pos {
                    // `IF [NOT] EXISTS` keeps the table position alive.
                    if matches!(upper.as_str(), "IF" | "NOT" | "EXISTS") {
                        continue;
                    }
                    check_scoped(plugin_id, &prefix, w)?;
                    table_pos = false;
                    continue;
                }
                if alter_rename && upper == "TO" {
                    table_pos = true; // ALTER … RENAME TO <new-name>
                    continue;
                }
                if matches!(upper.as_str(), "FROM" | "JOIN" | "INTO" | "UPDATE" | "TABLE") {
                    table_pos = true;
                }
            }
            Tok::Quoted(_) => {
                if table_pos {
                    return Err(ApiError::bad_request(
                        "plugin SQL: quoted table names are not permitted",
                    ));
                }
                table_pos = false;
            }
            Tok::Punct(_) => table_pos = false,
        }
    }

    Ok(trimmed.trim_end_matches(';').to_string())
}

fn check_scoped(plugin_id: &str, prefix: &str, ident: &str) -> Result<(), ApiError> {
    let ok = ident.starts_with(prefix)
        && ident.len() > prefix.len()
        && ident
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "plugin SQL: table '{ident}' is outside this plugin's scope (tables must be named {plugin_id}_<name>)"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(sql: &str) {
        validate_scoped_sql("calendar", sql)
            .unwrap_or_else(|e| panic!("{sql:?} rejected: {e}"));
    }

    fn rejected(sql: &str) {
        assert!(
            validate_scoped_sql("calendar", sql).is_err(),
            "{sql:?} should have been rejected"
        );
    }

    #[test]
    fn allows_scoped_statements() {
        ok("SELECT * FROM calendar_events");
        ok("INSERT INTO calendar_events (title) VALUES ('x')");
        ok("INSERT INTO calendar_events VALUES ('x', 1)");
        ok("UPDATE calendar_events SET title = 'y' WHERE id = 1");
        ok("DELETE FROM calendar_events WHERE id = 2");
        ok("CREATE TABLE IF NOT EXISTS calendar_events (id INTEGER PRIMARY KEY)");
        ok("DROP TABLE IF EXISTS calendar_events");
        ok("ALTER TABLE calendar_events RENAME TO calendar_events_old");
        ok("SELECT e.title FROM calendar_events e JOIN calendar_tags t ON t.id = e.tag_id");
        ok("SELECT COUNT(*) FROM calendar_events");
        ok("SELECT * FROM calendar_events WHERE title = 'FROM users'");
    }

    #[test]
    fn rejects_out_of_scope_tables() {
        rejected("SELECT * FROM users");
        rejected("SELECT * FROM sqlite_master");
        rejected("DROP TABLE notes");
        rejected("INSERT INTO calendarEvil VALUES (1)");
        rejected("SELECT * FROM calendar__events");
        rejected("ALTER TABLE calendar_events RENAME TO users");
        rejected("UPDATE users SET x = 1");
    }

    #[test]
    fn rejects_smuggling() {
        rejected("SELECT 1; DROP TABLE users");
        rejected("SELECT 1;");
        rejected("ATTACH DATABASE 'x' AS y");
        rejected("PRAGMA journal_mode = DELETE");
        rejected("VACUUM");
        rejected("EXPLAIN SELECT 1");
        rejected("BEGIN");
        rejected("SELECT * FROM \"users\"");
        rejected("SELECT 'unterminated");
        rejected("SELECT /* open");
        rejected("CREATE INDEX calendar_i ON calendar_events (title)");
        rejected("CREATE TRIGGER calendar_t AFTER INSERT ON calendar_events BEGIN SELECT 1; END");
        rejected("DROP INDEX calendar_i");
        rejected("SELECT * FROM users -- harmless comment");
        rejected("SELECT 1 UNION SELECT * FROM users");
        // UNION itself is fine on scoped tables only:
        ok("SELECT title FROM calendar_events UNION SELECT title FROM calendar_old");
    }

    #[test]
    fn length_and_empty_guards() {
        rejected("");
        rejected("   ");
        rejected(&format!("SELECT '{}'", "x".repeat(9_000)));
    }
}

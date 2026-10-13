// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Per-account login lockout. The per-client rate limit slows one address;
//! this slows guessing against one username from many addresses. Counts are
//! kept in memory and reset on restart, which is the same trade-off as the
//! rate limiter.
//!
//! The lock applies to any name, including names with no account, so a
//! locked response does not reveal which accounts exist.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Failed attempts in a row before the name is locked.
pub const MAX_FAILURES: u32 = 10;
/// How long a locked name refuses every attempt.
pub const LOCK_FOR: Duration = Duration::from_secs(15 * 60);
/// Upper bound on tracked names. Past it the table is cleared, which lets an
/// attacker forget counts but cannot exhaust memory.
const MAX_TRACKED: usize = 10_000;

#[derive(Clone, Copy)]
struct Entry {
    failures: u32,
    locked_until: Option<Instant>,
}

static TABLE: Mutex<Option<HashMap<String, Entry>>> = Mutex::new(None);

fn key(username: &str) -> String {
    username.trim().to_ascii_lowercase()
}

fn with_table<T>(f: impl FnOnce(&mut HashMap<String, Entry>) -> T) -> T {
    let mut guard = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    let table = guard.get_or_insert_with(HashMap::new);
    f(table)
}

/// `Some(remaining)` when the name is locked, `None` when attempts may proceed.
pub fn locked_for(username: &str) -> Option<Duration> {
    let name = key(username);
    with_table(|table| {
        let entry = table.get_mut(&name)?;
        let until = entry.locked_until?;
        let now = Instant::now();
        if until > now {
            Some(until - now)
        } else {
            // The lock has run out: start counting again from zero.
            *entry = Entry {
                failures: 0,
                locked_until: None,
            };
            None
        }
    })
}

/// Count a failed attempt; the name locks at `MAX_FAILURES`.
pub fn record_failure(username: &str) {
    let name = key(username);
    let now = Instant::now();
    with_table(|table| {
        if table.len() >= MAX_TRACKED && !table.contains_key(&name) {
            table.clear();
        }
        let entry = table.entry(name).or_insert(Entry {
            failures: 0,
            locked_until: None,
        });
        entry.failures += 1;
        if entry.failures >= MAX_FAILURES {
            entry.locked_until = Some(now + LOCK_FOR);
        }
    });
}

/// A successful login clears the count.
pub fn record_success(username: &str) {
    let name = key(username);
    with_table(|table| {
        table.remove(&name);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // The table is process-wide, so each test uses its own name.

    #[test]
    fn locks_after_max_failures_and_is_case_insensitive() {
        let name = "lockout-test-one";
        for _ in 0..MAX_FAILURES - 1 {
            record_failure(name);
        }
        assert!(locked_for(name).is_none());
        record_failure(name);
        assert!(locked_for(&name.to_uppercase()).is_some());
    }

    #[test]
    fn success_clears_the_count() {
        let name = "lockout-test-two";
        for _ in 0..MAX_FAILURES - 1 {
            record_failure(name);
        }
        record_success(name);
        record_failure(name);
        assert!(locked_for(name).is_none());
    }

    #[test]
    fn unknown_names_lock_the_same_way() {
        let name = "lockout-test-three-never-created";
        for _ in 0..MAX_FAILURES {
            record_failure(name);
        }
        let left = locked_for(name).expect("locked");
        assert!(left <= LOCK_FOR && left > LOCK_FOR - Duration::from_secs(5));
    }
}

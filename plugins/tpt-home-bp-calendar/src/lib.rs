// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Example plugin: a list of calendar events stored in `calendar_events`.
//!
//! Routes (below `/api/calendar`):
//! - `GET /events` — up to 200 events, soonest first
//! - `POST /events` — body `{"title": "...", "starts_at": <unix seconds>}`
//! - `DELETE /events?id=<n>` — remove one event

use serde::Deserialize;
use serde_json::json;
use tpt_home_plugin_sdk::{Request, Response, db_execute, db_query, export, sql_quote};

const MAX_TITLE_CHARS: usize = 200;

#[derive(Deserialize)]
struct NewEvent {
    title: String,
    starts_at: i64,
}

fn handle(req: Request) -> Response {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/events") => list_events(),
        ("POST", "/events") => create_event(&req),
        ("DELETE", "/events") => delete_event(&req),
        _ => Response::error(404, format!("no route for {} {}", req.method, req.path)),
    }
}

fn list_events() -> Response {
    match db_query("SELECT id, title, starts_at FROM calendar_events ORDER BY starts_at LIMIT 200") {
        Ok(rows) => Response::json(200, &rows),
        Err(e) => Response::error(500, e),
    }
}

fn create_event(req: &Request) -> Response {
    let event: NewEvent = match req.json() {
        Ok(event) => event,
        Err(e) => return Response::error(400, e),
    };
    let title = event.title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        return Response::error(400, format!("title must be 1-{MAX_TITLE_CHARS} characters"));
    }
    let sql = format!(
        "INSERT INTO calendar_events (title, starts_at) VALUES ({}, {})",
        sql_quote(title),
        event.starts_at
    );
    match db_execute(&sql) {
        Ok(_) => Response::json(201, &json!({ "ok": true })),
        Err(e) => Response::error(500, e),
    }
}

fn delete_event(req: &Request) -> Response {
    let Some(id) = req.query_param("id").and_then(|v| v.parse::<i64>().ok()) else {
        return Response::error(400, "query parameter 'id' must be an integer");
    };
    match db_execute(&format!("DELETE FROM calendar_events WHERE id = {id}")) {
        Ok(0) => Response::error(404, "no such event"),
        Ok(_) => Response::json(200, &json!({ "ok": true })),
        Err(e) => Response::error(500, e),
    }
}

export!(handle);

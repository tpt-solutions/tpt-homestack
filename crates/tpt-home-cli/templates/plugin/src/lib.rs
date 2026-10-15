// SPDX-License-Identifier: MIT OR Apache-2.0

//! <TITLE> plugin: a list of items stored in `<NAME>_items`.
//!
//! Routes (below `/api/<NAME>`):
//! - `GET /items` returns up to 200 items
//! - `POST /items` takes the body `{"title": "..."}`
//! - `DELETE /items?id=<n>` removes one item

use serde::Deserialize;
use serde_json::json;
use tpt_home_plugin_sdk::{Request, Response, db_execute, db_query, export, sql_quote};

const MAX_TITLE_CHARS: usize = 200;

#[derive(Deserialize)]
struct NewItem {
    title: String,
}

fn handle(req: Request) -> Response {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/items") => list_items(),
        ("POST", "/items") => create_item(&req),
        ("DELETE", "/items") => delete_item(&req),
        _ => Response::error(404, format!("no route for {} {}", req.method, req.path)),
    }
}

fn list_items() -> Response {
    match db_query("SELECT id, title FROM <NAME>_items ORDER BY id DESC LIMIT 200") {
        Ok(rows) => Response::json(200, &rows),
        Err(e) => Response::error(500, e),
    }
}

fn create_item(req: &Request) -> Response {
    let item: NewItem = match req.json() {
        Ok(item) => item,
        Err(e) => return Response::error(400, e),
    };
    let title = item.title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
        return Response::error(400, format!("title must be 1-{MAX_TITLE_CHARS} characters"));
    }
    let sql = format!("INSERT INTO <NAME>_items (title) VALUES ({})", sql_quote(title));
    match db_execute(&sql) {
        Ok(_) => Response::json(201, &json!({ "ok": true })),
        Err(e) => Response::error(500, e),
    }
}

fn delete_item(req: &Request) -> Response {
    let Some(id) = req.query_param("id").and_then(|v| v.parse::<i64>().ok()) else {
        return Response::error(400, "query parameter 'id' must be an integer");
    };
    match db_execute(&format!("DELETE FROM <NAME>_items WHERE id = {id}")) {
        Ok(0) => Response::error(404, "no such item"),
        Ok(_) => Response::json(200, &json!({ "ok": true })),
        Err(e) => Response::error(500, e),
    }
}

export!(handle);

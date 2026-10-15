// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Tools for AI agents. Each one calls the route it names, so the same checks
//! (login, token scope, rate limit) apply as for the notes pages.

use tpt_home_core::McpToolEntry;

const NOTE_INPUT: &str = r#"{"type":"object","properties":{"title":{"type":"string"},"body":{"type":"string","description":"Markdown text"},"tags":{"type":"array","items":{"type":"string"}}},"required":["title"]}"#;

tpt_home_core::inventory::submit! {
    McpToolEntry {
        blueprint: env!("CARGO_PKG_NAME"),
        name: "notes_list",
        description: "List notes, newest first. Optional q (full-text search) and tag filter.",
        input_schema: r#"{"type":"object","properties":{"q":{"type":"string"},"tag":{"type":"string"}}}"#,
        method: "GET",
        path: "/api/notes",
        role: None,
    }
}

tpt_home_core::inventory::submit! {
    McpToolEntry {
        blueprint: env!("CARGO_PKG_NAME"),
        name: "notes_create",
        description: "Create a note with a title, Markdown body and tags.",
        input_schema: NOTE_INPUT,
        method: "POST",
        path: "/api/notes",
        role: None,
    }
}

tpt_home_core::inventory::submit! {
    McpToolEntry {
        blueprint: env!("CARGO_PKG_NAME"),
        name: "notes_get",
        description: "Get one note by id, with its tags.",
        input_schema: r#"{"type":"object","properties":{"id":{"type":"integer"}},"required":["id"]}"#,
        method: "GET",
        path: "/api/notes/{id}",
        role: None,
    }
}

tpt_home_core::inventory::submit! {
    McpToolEntry {
        blueprint: env!("CARGO_PKG_NAME"),
        name: "notes_update",
        description: "Replace a note's title, body and tags.",
        input_schema: r#"{"type":"object","properties":{"id":{"type":"integer"},"title":{"type":"string"},"body":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}}},"required":["id","title"]}"#,
        method: "PUT",
        path: "/api/notes/{id}",
        role: None,
    }
}

tpt_home_core::inventory::submit! {
    McpToolEntry {
        blueprint: env!("CARGO_PKG_NAME"),
        name: "notes_delete",
        description: "Delete a note by id. This cannot be undone.",
        input_schema: r#"{"type":"object","properties":{"id":{"type":"integer"}},"required":["id"]}"#,
        method: "DELETE",
        path: "/api/notes/{id}",
        role: None,
    }
}

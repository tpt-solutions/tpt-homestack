// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Tools for AI agents over tracking tables. Each one calls the route it names,
//! so login, token scope and rate limits apply as they do for the pages.

use tpt_home_core::McpToolEntry;

tpt_home_core::inventory::submit! {
    McpToolEntry {
        blueprint: env!("CARGO_PKG_NAME"),
        name: "track_tables_list",
        description: "List the tracking tables and their fields.",
        input_schema: r#"{"type":"object","properties":{}}"#,
        method: "GET",
        path: "/api/track/tables",
        role: None,
    }
}

tpt_home_core::inventory::submit! {
    McpToolEntry {
        blueprint: env!("CARGO_PKG_NAME"),
        name: "track_rows_list",
        description: "List the rows of one tracking table. Extra arguments are filters, named by field.",
        input_schema: r#"{"type":"object","properties":{"id":{"type":"integer","description":"Table id"}},"required":["id"],"additionalProperties":true}"#,
        method: "GET",
        path: "/api/track/tables/{id}/rows",
        role: None,
    }
}

tpt_home_core::inventory::submit! {
    McpToolEntry {
        blueprint: env!("CARGO_PKG_NAME"),
        name: "track_row_add",
        description: "Add a row to a tracking table. Other arguments are the field values, named by field.",
        input_schema: r#"{"type":"object","properties":{"id":{"type":"integer","description":"Table id"}},"required":["id"],"additionalProperties":true}"#,
        method: "POST",
        path: "/api/track/tables/{id}/rows",
        role: None,
    }
}

tpt_home_core::inventory::submit! {
    McpToolEntry {
        blueprint: env!("CARGO_PKG_NAME"),
        name: "track_row_update",
        description: "Change some field values of a row. Other arguments are the new values, named by field.",
        input_schema: r#"{"type":"object","properties":{"id":{"type":"integer","description":"Table id"},"row":{"type":"integer","description":"Row id"}},"required":["id","row"],"additionalProperties":true}"#,
        method: "PATCH",
        path: "/api/track/tables/{id}/rows/{row}",
        role: None,
    }
}

tpt_home_core::inventory::submit! {
    McpToolEntry {
        blueprint: env!("CARGO_PKG_NAME"),
        name: "track_row_delete",
        description: "Delete a row from a tracking table. This cannot be undone.",
        input_schema: r#"{"type":"object","properties":{"id":{"type":"integer","description":"Table id"},"row":{"type":"integer","description":"Row id"}},"required":["id","row"]}"#,
        method: "DELETE",
        path: "/api/track/tables/{id}/rows/{row}",
        role: None,
    }
}

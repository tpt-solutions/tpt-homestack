// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Plain view models consumed by the shell renderers. Keeping these as dumb
//! structs means the render layer never touches the database or auth types.

/// The signed-in user as displayed by the shell.
pub struct UserView {
    pub username: String,
    pub role: String,
}

/// One row of the admin · users table.
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub disabled: bool,
}

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Shared page components: buttons, form fields, tables, lists, badges,
//! messages, modal dialogs and toasts. Each builder returns an HTML string.
//!
//! Text arguments are escaped here. Table cells and list items are the
//! exception: they are HTML, so build each cell with [`text`] (escapes) or
//! with a builder from this module. Event handlers use `data-click`,
//! `data-submit` and `data-change` (see `DISPATCH_JS`), never inline
//! `on*` attributes, which the Content-Security-Policy refuses.

use crate::esc;

/// Escape plain text for use as a table cell or list item.
pub fn text(s: &str) -> String {
    esc(s)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Danger,
}

/// A button that calls the global function `action` through `data-click`.
/// `action` is `fn` or `fn:arg:arg` (see the dispatcher).
pub fn button(label: &str, action: &str, kind: ButtonKind) -> String {
    let class = match kind {
        ButtonKind::Primary => "",
        ButtonKind::Secondary => " class=\"secondary\"",
        ButtonKind::Danger => " class=\"danger\"",
    };
    format!(
        "<button type=\"button\"{class} data-click=\"{}\">{}</button>",
        esc(action),
        esc(label)
    )
}

/// A labelled text input. `id` ties the label to the input.
pub fn field(label: &str, id: &str, input_type: &str, value: &str) -> String {
    format!(
        "<label for=\"{id}\">{label}</label>\n<input id=\"{id}\" type=\"{ty}\" value=\"{value}\">",
        id = esc(id),
        label = esc(label),
        ty = esc(input_type),
        value = esc(value),
    )
}

/// A labelled select. `options` are `(value, text)`; the one whose value is
/// `selected` is marked.
pub fn select(label: &str, id: &str, options: &[(&str, &str)], selected: &str) -> String {
    let opts: String = options
        .iter()
        .map(|(value, text)| {
            let mark = if *value == selected { " selected" } else { "" };
            format!(
                "<option value=\"{}\"{mark}>{}</option>",
                esc(value),
                esc(text)
            )
        })
        .collect();
    format!(
        "<label for=\"{id}\">{label}</label>\n<select id=\"{id}\">{opts}</select>",
        id = esc(id),
        label = esc(label),
    )
}

/// A form that calls `action` through `data-submit` when submitted. `body`
/// is HTML (fields and buttons from this module).
pub fn form(action: &str, body: &str) -> String {
    format!("<form data-submit=\"{}\">\n{body}\n</form>", esc(action))
}

/// A table. `headers` are plain text; each row is a list of cells, already
/// HTML (use [`text`] for plain values). Shows `empty` when there are no rows.
pub fn table(headers: &[&str], rows: &[Vec<String>], empty: &str) -> String {
    if rows.is_empty() {
        return format!("<p class=\"muted\">{}</p>", esc(empty));
    }
    let head: String = headers
        .iter()
        .map(|h| format!("<th>{}</th>", esc(h)))
        .collect();
    let body: String = rows
        .iter()
        .map(|cells| {
            let tds: String = cells.iter().map(|c| format!("<td>{c}</td>")).collect();
            format!("<tr>{tds}</tr>")
        })
        .collect();
    format!(
        "<div style=\"overflow-x:auto\"><table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table></div>"
    )
}

/// A bulleted list. Items are HTML (use [`text`] for plain values).
pub fn list(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        return format!("<p class=\"muted\">{}</p>", esc(empty));
    }
    let lis: String = items.iter().map(|i| format!("<li>{i}</li>")).collect();
    format!("<ul>{lis}</ul>")
}

/// A pill-shaped status label. `warn` marks something that needs attention.
pub fn badge(label: &str, warn: bool) -> String {
    let class = if warn { "badge warn" } else { "badge" };
    format!("<span class=\"{class}\">{}</span>", esc(label))
}

/// An inline message, announced to screen readers.
pub fn message(text: &str, ok: bool) -> String {
    let class = if ok { "msg ok" } else { "msg error" };
    format!("<div class=\"{class}\" role=\"status\">{}</div>", esc(text))
}

/// A modal dialog with a title and an HTML body (build it from this module
/// or escape it with [`text`]). Open it with `openModal('<id>')`,
/// close it with the close button or Escape (the browser handles both).
pub fn modal(id: &str, title: &str, body: &str) -> String {
    format!(
        "<dialog id=\"{id}\" class=\"modal\" aria-labelledby=\"{id}-title\">\n\
         <div class=\"spread\"><h2 id=\"{id}-title\" style=\"margin:0\">{title}</h2>\
         <button type=\"button\" class=\"secondary\" data-click=\"closeModal:{id}\" aria-label=\"Close\">Close</button></div>\n\
         <div class=\"modal-body\">{body}</div>\n</dialog>",
        id = esc(id),
        title = esc(title),
        body = body,
    )
}

/// The live region toasts appear in. Every page shell includes one.
pub fn toast_region() -> String {
    "<div id=\"toasts\" class=\"toasts\" role=\"status\" aria-live=\"polite\"></div>".to_string()
}

/// Client-side helpers for modals and toasts. Included in every page.
pub const COMPONENTS_JS: &str = r#"
function openModal(id) { const d = document.getElementById(id); if (d && !d.open) d.showModal(); }
function closeModal(id) { const d = document.getElementById(id); if (d && d.open) d.close(); }
function toast(text, ok) {
  const region = document.getElementById('toasts');
  if (!region) return;
  const t = document.createElement('div');
  t.className = 'toast ' + (ok ? 'ok' : 'error');
  t.textContent = text;
  region.appendChild(t);
  setTimeout(() => t.remove(), ok ? 4000 : 7000);
}
"#;

/// Styles for the components above.
pub const COMPONENTS_CSS: &str = r#"
dialog.modal { background:var(--card); color:var(--text); border:1px solid var(--line);
  border-radius:12px; padding:18px; width:min(560px, 92vw); }
dialog.modal::backdrop { background:rgba(0,0,0,.35); }
.modal-body { margin-top:12px; }
.toasts { position:fixed; right:16px; bottom:16px; display:flex; flex-direction:column;
  gap:8px; z-index:50; max-width:min(420px, 92vw); }
.toast { padding:10px 14px; border-radius:8px; font-size:14px; box-shadow:0 4px 14px rgba(0,0,0,.18); }
.toast.ok { background:#dcfce7; color:var(--ok); }
.toast.error { background:#fee2e2; color:var(--danger); }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_escaped_everywhere() {
        let evil = "<img src=x onerror=alert(1)>";
        assert!(!button(evil, "go", ButtonKind::Primary).contains("<img"));
        assert!(!field(evil, "f", "text", evil).contains("<img"));
        assert!(!message(evil, false).contains("<img"));
        assert!(!badge(evil, true).contains("<img"));
        assert!(!modal("m", evil, &text(evil)).contains("<img"));
        assert!(!table(&[evil], &[vec![text(evil)]], evil).contains("<img"));
        assert!(!list(&[text(evil)], evil).contains("<img"));
    }

    #[test]
    fn components_use_data_handlers_not_inline_events() {
        let all = [
            button("Save", "save", ButtonKind::Primary),
            form("save", &field("Name", "n", "text", "")),
            modal("m", "Title", "body"),
            select("Role", "r", &[("user", "user")], "user"),
        ];
        for html in all {
            assert!(
                !html.contains(" onclick=") && !html.contains(" onsubmit="),
                "{html}"
            );
        }
        assert!(button("x", "go:1", ButtonKind::Danger).contains("data-click=\"go:1\""));
        assert!(form("save", "").contains("data-submit=\"save\""));
    }

    #[test]
    fn table_shows_the_empty_text_without_rows() {
        assert!(table(&["A"], &[], "Nothing yet").contains("Nothing yet"));
        let t = table(&["Name"], &[vec![text("a")]], "none");
        assert!(t.contains("<th>Name</th>") && t.contains("<td>a</td>"));
    }

    #[test]
    fn select_marks_only_the_selected_option() {
        let s = select(
            "Role",
            "r",
            &[("user", "User"), ("admin", "Admin")],
            "admin",
        );
        assert!(s.contains("<option value=\"admin\" selected>"));
        assert!(!s.contains("<option value=\"user\" selected>"));
    }

    #[test]
    fn modal_closes_through_the_dispatcher() {
        assert!(modal("pw", "Change", "").contains("data-click=\"closeModal:pw\""));
    }

    #[test]
    fn users_page_uses_a_modal_and_toasts_not_blocking_dialogs() {
        let html = crate::users_admin_content(&[]);
        assert!(html.contains("id=\"pw-modal\""));
        assert!(!html.contains("alert("));
        assert!(!html.contains("prompt("));
    }

    #[test]
    fn every_page_has_the_toast_region_and_helpers() {
        let user = crate::UserView {
            username: "a".into(),
            role: "admin".into(),
        };
        let html = crate::page("T", &[], "/x", &user, "");
        assert!(html.contains("id=\"toasts\""));
        assert!(html.contains("function toast("));
        assert!(html.contains("function openModal("));
    }
}

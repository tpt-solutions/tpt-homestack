// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Server-rendered UI shell for tpt-homestack.
//!
//! Pure functions producing HTML strings: no framework, no runtime deps, and
//! a few KiB of binary. This is the swap-in point for the Leptos shell
//! (Phase 3): the api crate only calls [`page`], [`login_page`] and friends,
//! so the rendering backend can change without touching routing.

mod view;

pub use view::{UserRow, UserView};

/// Escape text for safe HTML interpolation. Every dynamic value rendered by
/// this crate passes through here.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Base stylesheet. Light/dark via `prefers-color-scheme`, zero JS.
pub const CSS: &str = r#"
:root { --bg:#f5f6f8; --card:#ffffff; --text:#1c2430; --muted:#5c6672;
        --line:#dde2e8; --accent:#2563eb; --accent-soft:#dbeafe;
        --danger:#dc2626; --ok:#16a34a; }
@media (prefers-color-scheme: dark) {
  :root { --bg:#12161c; --card:#1a212b; --text:#e6eaf0; --muted:#98a2ae;
          --line:#2a3340; --accent:#3b82f6; --accent-soft:#1e3a5f;
          --danger:#ef4444; --ok:#22c55e; }
}
* { box-sizing:border-box; }
body { margin:0; font:15px/1.5 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;
       background:var(--bg); color:var(--text); }
a { color:var(--accent); text-decoration:none; }
a:hover { text-decoration:underline; }
.layout { display:flex; min-height:100vh; }
.sidebar { width:220px; flex:0 0 220px; background:var(--card);
           border-right:1px solid var(--line); padding:16px; }
.sidebar .brand { font-weight:700; font-size:17px; margin-bottom:18px; }
.sidebar nav a { display:block; padding:8px 10px; border-radius:8px;
                 color:var(--text); margin-bottom:2px; }
.sidebar nav a:hover { background:var(--accent-soft); text-decoration:none; }
.sidebar nav a.active { background:var(--accent-soft); color:var(--accent); font-weight:600; }
.sidebar .who { margin-top:22px; font-size:13px; color:var(--muted); }
.sidebar .who a { display:inline-block; margin-right:8px; }
.main { flex:1; padding:26px 32px; max-width:980px; }
h1 { font-size:22px; margin:0 0 18px; }
h2 { font-size:16px; margin:22px 0 8px; }
.card { background:var(--card); border:1px solid var(--line); border-radius:12px;
        padding:18px; margin-bottom:16px; }
.grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(230px,1fr)); gap:14px; }
.muted { color:var(--muted); font-size:13px; }
.badge { display:inline-block; padding:1px 8px; border-radius:99px; font-size:12px;
         background:var(--accent-soft); color:var(--accent); }
.badge.warn { background:#fee2e2; color:var(--danger); }
button, .btn { background:var(--accent); color:#fff; border:0; border-radius:8px;
        padding:8px 14px; font-size:14px; cursor:pointer; }
button.secondary { background:transparent; color:var(--accent); border:1px solid var(--accent); }
button.danger { background:transparent; color:var(--danger); border:1px solid var(--danger); }
input, select, textarea { background:var(--bg); color:var(--text); border:1px solid var(--line);
        border-radius:8px; padding:8px 10px; font-size:14px; width:100%; }
label { display:block; font-size:13px; color:var(--muted); margin:10px 0 4px; }
table { border-collapse:collapse; width:100%; }
th, td { text-align:left; padding:8px 10px; border-bottom:1px solid var(--line); font-size:14px; }
th { color:var(--muted); font-weight:600; }
.center { max-width:380px; margin:12vh auto 0; }
.msg { padding:10px 12px; border-radius:8px; margin-bottom:12px; font-size:14px; }
.msg.error { background:#fee2e2; color:var(--danger); }
.msg.ok { background:#dcfce7; color:var(--ok); }
.row { display:flex; gap:10px; align-items:center; flex-wrap:wrap; }
.spread { display:flex; justify-content:space-between; align-items:center; gap:10px; }
code, pre { font-family:ui-monospace,SFMono-Regular,Consolas,monospace; }
pre.md { background:var(--bg); border:1px solid var(--line); border-radius:8px;
         padding:14px; overflow-x:auto; }
#mobile-toggle { display:none; }
@media (max-width: 760px) {
  .layout { flex-direction:column; }
  .sidebar { width:auto; flex:none; border-right:0; border-bottom:1px solid var(--line); }
  .main { padding:16px; }
}
"#;

fn shell_open(title: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{} · tpt-homestack</title>\n<style>{CSS}</style>\n</head>\n<body>",
        esc(title)
    )
}

fn sidebar(nav: &[(String, String)], active: &str, user: &UserView) -> String {
    let mut links = String::from("<nav>\n<a href=\"/dashboard\">Dashboard</a>\n");
    for (label, path) in nav {
        let class = if path == active {
            " class=\"active\""
        } else {
            ""
        };
        links.push_str(&format!("<a href=\"{path}\"{class}>{}</a>\n", esc(label)));
    }
    if user.role == "admin" {
        let class = if active == "/admin/users" {
            " class=\"active\""
        } else {
            ""
        };
        links.push_str(&format!(
            "<a href=\"/admin/users\"{class}>Admin · Users</a>\n"
        ));
    }
    links.push_str("</nav>");
    format!(
        "<aside class=\"sidebar\">\n<div class=\"brand\">tpt-homestack</div>\n{links}\n\
         <div class=\"who\">signed in as <strong>{}</strong> <span class=\"badge\">{}</span><br>\
         <a href=\"/settings\">Settings</a><a href=\"#\" onclick=\"logout();return false\">Sign out</a></div>\n</aside>",
        esc(&user.username),
        esc(&user.role)
    )
}

/// Shell page: sidebar + `content` for an authenticated user.
pub fn page(
    title: &str,
    nav: &[(String, String)],
    active: &str,
    user: &UserView,
    content: &str,
) -> String {
    format!(
        "{}\n<div class=\"layout\">\n{}\n<main class=\"main\">\n<h1>{}</h1>\n{}\n</main>\n</div>\n<script>\n\
         async function postJson(url, body) {{\n  const r = await fetch(url, {{method:'POST', headers:{{'Content-Type':'application/json'}}, body: JSON.stringify(body)}});\n  const j = await r.json().catch(() => ({{}}));\n  if (!r.ok) throw new Error(j.error || r.statusText);\n  return j;\n}}\n\
         async function logout() {{ await postJson('/api/auth/logout', {{}}); location.href = '/login'; }}\n</script>\n</body>\n</html>",
        shell_open(title),
        sidebar(nav, active, user),
        esc(title),
        content
    )
}

/// Standalone (no-shell) page used for login and first-run setup.
pub fn bare_page(title: &str, body: &str) -> String {
    format!(
        "{}\n<div class=\"center\">\n<div class=\"brand\" style=\"font-weight:700;font-size:20px;margin-bottom:16px\">tpt-homestack</div>\n{}\n</div>\n</body>\n</html>",
        shell_open(title),
        body
    )
}

/// Login page; `error` renders a red banner after a failed attempt.
pub fn login_page(error: Option<&str>) -> String {
    let err = error
        .map(|e| format!("<div class=\"msg error\">{}</div>", esc(e)))
        .unwrap_or_default();
    let body = format!(
        "<div class=\"card\">\n{err}\n<h1 style=\"margin-top:0\">Sign in</h1>\n\
         <form onsubmit=\"return doLogin(event)\">\n\
         <label for=\"u\">Username</label><input id=\"u\" autocomplete=\"username\" required>\n\
         <label for=\"p\">Password</label><input id=\"p\" type=\"password\" autocomplete=\"current-password\" required>\n\
         <div style=\"margin-top:14px\"><button type=\"submit\">Sign in</button></div>\n</form>\n</div>\n\
         <script>\nasync function doLogin(ev) {{\n  ev.preventDefault();\n  try {{\n    await postJson('/api/auth/login', {{username: document.getElementById('u').value, password: document.getElementById('p').value}});\n    location.href = '/dashboard';\n  }} catch (e) {{ location.href = '/login?error=' + encodeURIComponent(e.message); }}\n  return false;\n}}\n</script>"
    );
    bare_page("Sign in", &body)
}

/// First-run setup page: create the initial admin account.
pub fn setup_page(error: Option<&str>) -> String {
    let err = error
        .map(|e| format!("<div class=\"msg error\">{}</div>", esc(e)))
        .unwrap_or_default();
    let body = format!(
        "<div class=\"card\">\n{err}\n<h1 style=\"margin-top:0\">Welcome</h1>\n\
         <p class=\"muted\">This instance has no users yet. Create the first admin account.</p>\n\
         <form onsubmit=\"return doSetup(event)\">\n\
         <label for=\"u\">Admin username</label><input id=\"u\" autocomplete=\"username\" required>\n\
         <label for=\"p\">Password (min 8 chars)</label><input id=\"p\" type=\"password\" autocomplete=\"new-password\" required>\n\
         <div style=\"margin-top:14px\"><button type=\"submit\">Create admin</button></div>\n</form>\n</div>\n\
         <script>\nasync function doSetup(ev) {{\n  ev.preventDefault();\n  try {{\n    await postJson('/api/setup', {{username: document.getElementById('u').value, password: document.getElementById('p').value}});\n    location.href = '/dashboard';\n  }} catch (e) {{ location.href = '/setup?error=' + encodeURIComponent(e.message); }}\n  return false;\n}}\n</script>"
    );
    bare_page("Setup", &body)
}

/// Dashboard: one card per installed blueprint.
pub fn dashboard_content(blueprints: &[(&str, &str)]) -> String {
    if blueprints.is_empty() {
        return "<div class=\"card\"><p class=\"muted\">No blueprints installed yet. \
                Create one with <code>tpt-home new my-app</code> and recompile.</p></div>"
            .to_string();
    }
    let cards: String = blueprints
        .iter()
        .map(|(id, name)| {
            format!(
                "<div class=\"card\"><div class=\"spread\"><strong>{}</strong> <span class=\"badge\">{}</span></div>\
                 <p class=\"muted\">Blueprint</p></div>",
                esc(name),
                esc(id)
            )
        })
        .collect();
    format!("<div class=\"grid\">{cards}</div>")
}

/// Settings page: change your own password.
pub fn settings_content() -> String {
    "<div class=\"card\" style=\"max-width:420px\">\n\
     <h2 style=\"margin-top:0\">Change password</h2>\n\
     <form onsubmit=\"return doPwChange(event)\">\n\
     <label for=\"cur\">Current password</label><input id=\"cur\" type=\"password\" autocomplete=\"current-password\" required>\n\
     <label for=\"new\">New password (min 8 chars)</label><input id=\"new\" type=\"password\" autocomplete=\"new-password\" required>\n\
     <div id=\"pwmsg\"></div>\n\
     <div style=\"margin-top:14px\"><button type=\"submit\">Update password</button></div>\n</form>\n</div>\n\
     <script>\nasync function doPwChange(ev) {{\n  ev.preventDefault();\n  const msg = document.getElementById('pwmsg');\n  try {{\n    await postJson('/api/auth/change-password', {{current: document.getElementById('cur').value, new_password: document.getElementById('new').value}});\n    msg.innerHTML = '<div class=\"msg ok\">Password changed. Sign in again…</div>';\n    setTimeout(() => location.href = '/login', 1200);\n  }} catch (e) {{ msg.innerHTML = '<div class=\"msg error\">' + e.message + '</div>'; }}\n  return false;\n}}\n</script>"
        .to_string()
}

/// Admin · users page.
pub fn users_admin_content(users: &[UserRow]) -> String {
    let rows: String = users
        .iter()
        .map(|u| {
            let status = if u.disabled {
                "<span class=\"badge warn\">disabled</span>".to_string()
            } else {
                "<span class=\"badge\">active</span>".to_string()
            };
            let role_sel = format!(
                "<select onchange=\"setRole({}, this.value)\">{}{} </select>",
                u.id,
                if u.role == "admin" {
                    "<option value=\"admin\" selected>admin</option><option value=\"user\">user</option>"
                } else {
                    "<option value=\"admin\">admin</option><option value=\"user\" selected>user</option>"
                },
                ""
            );
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
                 <td class=\"row\">{}<button class=\"secondary\" onclick=\"resetPw({}, prompt('new password (min 8 chars)'))\">Reset pw</button>\
                 <button class=\"danger\" onclick=\"delUser({})\">Delete</button></td></tr>",
                u.id,
                esc(&u.username),
                status,
                role_sel,
                if u.disabled {
                    format!("<button class=\"secondary\" onclick=\"setDisabled({}, false)\">Enable</button>", u.id)
                } else {
                    format!("<button class=\"secondary\" onclick=\"setDisabled({}, true)\">Disable</button>", u.id)
                },
                u.id,
                u.id
            )
        })
        .collect();
    format!(
        "<div class=\"card\">\n<h2 style=\"margin-top:0\">Create user</h2>\n\
         <form onsubmit=\"return doCreate(event)\">\n\
         <div class=\"row\"><input id=\"nu\" placeholder=\"username\" style=\"max-width:200px\" required>\n\
         <input id=\"np\" type=\"password\" placeholder=\"password\" style=\"max-width:200px\" required>\n\
         <select id=\"nr\"><option value=\"user\">user</option><option value=\"admin\">admin</option></select>\n\
         <button type=\"submit\">Create</button></div>\n</form>\n</div>\n\
         <div class=\"card\">\n<table>\n<thead><tr><th>#</th><th>User</th><th>Status</th><th>Role</th><th>Actions</th></tr></thead>\n\
         <tbody>{rows}</tbody>\n</table>\n</div>\n\
         <script>\nasync function api(url, method, body) {{\n  const r = await fetch(url, {{method, headers:{{'Content-Type':'application/json'}}, body: body ? JSON.stringify(body) : undefined}});\n  const j = await r.json().catch(() => ({{}}));\n  if (!r.ok) throw new Error(j.error || r.statusText);\n  return j;\n}}\n\
         function reload() {{ location.reload(); }}\nasync function doCreate(ev) {{ ev.preventDefault(); try {{ await api('/api/admin/users', 'POST', {{username: nu.value, password: np.value, role: nr.value}}); reload(); }} catch (e) {{ alert(e.message); }} return false; }}\n\
         async function setRole(id, role) {{ try {{ await api('/api/admin/users/' + id, 'PATCH', {{role}}); reload(); }} catch (e) {{ alert(e.message); }} }}\n\
         async function setDisabled(id, disabled) {{ try {{ await api('/api/admin/users/' + id, 'PATCH', {{disabled}}); reload(); }} catch (e) {{ alert(e.message); }} }}\n\
         async function resetPw(id, pw) {{ if (!pw || pw.length < 8) return; try {{ await api('/api/admin/users/' + id, 'PATCH', {{password: pw}}); alert('password updated'); }} catch (e) {{ alert(e.message); }} }}\n\
         async function delUser(id) {{ if (!confirm('Delete user ' + id + '?')) return; try {{ await api('/api/admin/users/' + id, 'DELETE'); reload(); }} catch (e) {{ alert(e.message); }} }}\n</script>"
    )
}

/// Text for one JSON cell: strings as-is, null as empty, anything else as JSON.
fn cell_text(value: Option<&serde_json::Value>) -> String {
    match value.cloned().unwrap_or(serde_json::Value::Null) {
        serde_json::Value::String(s) => s,
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The `?id=` link to a row's detail page, when the row has an integer id.
fn detail_href(link: &str, row: &serde_json::Value) -> Option<String> {
    let id = row.get("id")?.as_i64()?;
    Some(format!("{}?id={id}", esc(link)))
}

/// Declarative plugin page: a read-only table rendered by the host from the
/// plugin's manifest-scoped database table. `rows` holds one JSON object per
/// row (plugin data; escaped at render time here). With `link`, the first
/// column links each row to its detail page.
pub fn plugin_table_page(
    columns: &[String],
    rows: &[serde_json::Value],
    api_base: &str,
    link: Option<&str>,
) -> String {
    let headers: String = columns
        .iter()
        .map(|c| format!("<th>{}</th>", esc(c)))
        .collect();
    let mut body = String::new();
    for row in rows {
        body.push_str("<tr>");
        for (i, col) in columns.iter().enumerate() {
            let text = esc(&cell_text(row.get(col)));
            match (i, link.and_then(|l| detail_href(l, row))) {
                (0, Some(href)) => {
                    body.push_str(&format!("<td><a href=\"{href}\">{text}</a></td>"))
                }
                _ => body.push_str(&format!("<td>{text}</td>")),
            }
        }
        body.push_str("</tr>\n");
    }
    format!(
        "<div class=\"card\">\n<table>\n<thead><tr>{headers}</tr></thead>\n<tbody>{body}</tbody>\n</table>\n\
         <p class=\"muted\">Rows served from the plugin's scoped table. Add data via \
         <code>POST {api_base}</code>.</p>\n</div>"
    )
}

/// Declarative list page: one item per row, the first column as its title and
/// the rest as a line of detail. Titles link to the detail page when `link` is set.
pub fn plugin_list_page(
    columns: &[String],
    rows: &[serde_json::Value],
    link: Option<&str>,
) -> String {
    let Some((title_col, meta_cols)) = columns.split_first() else {
        return plugin_error_page("list page has no columns");
    };
    if rows.is_empty() {
        return "<div class=\"card muted\">No items yet.</div>".to_string();
    }
    let mut items = String::new();
    for row in rows {
        let title = esc(&cell_text(row.get(title_col)));
        let title = match link.and_then(|l| detail_href(l, row)) {
            Some(href) => format!("<a href=\"{href}\">{title}</a>"),
            None => title,
        };
        let meta: Vec<String> = meta_cols
            .iter()
            .map(|c| esc(&cell_text(row.get(c))))
            .filter(|s| !s.is_empty())
            .collect();
        items.push_str(&format!(
            "<li><div>{title}</div><div class=\"muted\">{}</div></li>\n",
            meta.join(" · ")
        ));
    }
    format!("<div class=\"card\"><ul class=\"plugin-list\">\n{items}</ul></div>")
}

/// Declarative detail page: one row as label/value pairs. `row` is `None` when
/// no row has the requested id.
pub fn plugin_detail_page(columns: &[String], row: Option<&serde_json::Value>) -> String {
    let Some(row) = row else {
        return plugin_error_page("no item with that id");
    };
    let mut pairs = String::new();
    for col in columns {
        pairs.push_str(&format!(
            "<dt>{}</dt><dd>{}</dd>\n",
            esc(col),
            esc(&cell_text(row.get(col)))
        ));
    }
    format!("<div class=\"card\"><dl class=\"plugin-detail\">\n{pairs}</dl></div>")
}

/// Declarative plugin form page: an HTML form that posts JSON to the
/// plugin's API route (the plugin's wasm handles it).
pub fn plugin_form_page(fields: &[String], action: &str) -> String {
    let inputs: String = fields
        .iter()
        .map(|f| {
            format!(
                "<label for=\"pf_{}\">{}</label><input id=\"pf_{}\" name=\"{}\">",
                esc(f),
                esc(f),
                esc(f),
                esc(f)
            )
        })
        .collect();
    let field_names: Vec<String> = fields.iter().map(|f| format!("'{f}'")).collect();
    format!(
        "<div class=\"card\" style=\"max-width:480px\">\n\
         <form onsubmit=\"return doSubmit(event)\">\n{inputs}\n\
         <div style=\"margin-top:14px\"><button type=\"submit\">Submit</button></div>\n</form>\n\
         <div id=\"pmsg\"></div>\n</div>\n\
         <script>\nasync function doSubmit(ev) {{\n  ev.preventDefault();\n  const body = {{}};\n  for (const f of [{}]) body[f] = document.getElementById('pf_' + f).value;\n  const r = await fetch('{action}', {{method:'POST', headers:{{'Content-Type':'application/json'}}, body: JSON.stringify(body)}});\n  const j = await r.json().catch(() => ({{}}));\n  const msg = document.getElementById('pmsg');\n  msg.innerHTML = r.ok ? '<div class=\"msg ok\">saved</div>' : '<div class=\"msg error\">' + (j.error || r.statusText) + '</div>';\n  return false;\n}}\n</script>",
        field_names.join(", ")
    )
}

/// Plugin error page fragment.
pub fn plugin_error_page(message: &str) -> String {
    format!("<div class=\"msg error\">{}</div>", esc(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html() {
        assert_eq!(esc("<b>&\"'"), "&lt;b&gt;&amp;&quot;&#39;");
    }

    fn row(id: i64, label: &str) -> serde_json::Value {
        serde_json::json!({ "id": id, "label": label, "note": "" })
    }

    #[test]
    fn table_links_only_the_first_cell_to_the_detail_page() {
        let cols = vec!["label".to_string(), "note".to_string()];
        let html = plugin_table_page(&cols, &[row(7, "milk")], "/api/x", Some("/plugins/x/item"));
        assert!(
            html.contains("<a href=\"/plugins/x/item?id=7\">milk</a>"),
            "{html}"
        );
        assert_eq!(html.matches("<a ").count(), 1);
    }

    #[test]
    fn list_and_table_values_are_escaped() {
        let cols = vec!["label".to_string()];
        let html = plugin_list_page(
            &cols,
            &[row(1, "<script>x</script>")],
            Some("/plugins/x/item"),
        );
        assert!(!html.contains("<script>x"), "{html}");
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("href=\"/plugins/x/item?id=1\""));
    }

    #[test]
    fn list_without_rows_says_so_and_without_columns_errors() {
        assert!(plugin_list_page(&["label".to_string()], &[], None).contains("No items yet"));
        assert!(plugin_list_page(&[], &[row(1, "a")], None).contains("no columns"));
    }

    #[test]
    fn detail_shows_the_row_or_a_not_found_message() {
        let cols = vec!["label".to_string()];
        assert!(plugin_detail_page(&cols, Some(&row(1, "milk"))).contains("<dd>milk</dd>"));
        assert!(plugin_detail_page(&cols, None).contains("no item with that id"));
    }

    #[test]
    fn login_page_renders_error_safely() {
        let page = login_page(Some("<script>alert(1)</script>"));
        assert!(!page.contains("<script>alert(1)"));
        assert!(page.contains("&lt;script&gt;"));
    }

    #[test]
    fn page_includes_nav_and_user() {
        let user = UserView {
            username: "admin".into(),
            role: "admin".into(),
        };
        let p = page(
            "Dashboard",
            &[("Notes".to_string(), "/notes".to_string())],
            "/notes",
            &user,
            "<p>hi</p>",
        );
        assert!(p.contains("href=\"/notes\" class=\"active\""));
        assert!(p.contains("Admin · Users"));
        assert!(p.contains("<p>hi</p>"));
    }
}

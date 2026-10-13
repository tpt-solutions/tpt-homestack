// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Server-rendered UI shell for tpt-homestack.
//!
//! Pure functions producing HTML strings: no framework, no runtime deps, and
//! a few KiB of binary. This is the swap-in point for the Leptos shell
//! (Phase 3): the api crate only calls [`page`], [`login_page`] and friends,
//! so the rendering backend can change without touching routing.

pub mod components;
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

/// Event delegation for every page. Elements name their handler in
/// `data-click`, `data-submit` or `data-change`, as `fn` or `fn:arg:arg`. Whole
/// numbers in the arguments are passed as numbers. The named global function is
/// called with those arguments, then the event and the element. Returning
/// `false` cancels the default action. Pages use this
/// instead of inline `onclick=` attributes, so the Content-Security-Policy can
/// refuse inline handlers.
pub const DISPATCH_JS: &str = r#"
(function () {
  function run(attr, e) {
    const el = e.target instanceof Element ? e.target.closest('[' + attr + ']') : null;
    if (!el) return;
    const parts = el.getAttribute(attr).split(':');
    const fn = window[parts[0]];
    if (typeof fn !== 'function') return;
    // Whole numbers arrive as numbers, as they did from inline handlers, so
    // comparisons like `t.id === id` still hold.
    const args = parts.slice(1).map((a) => (/^-?\d+$/.test(a) ? Number(a) : a));
    if (fn.apply(el, args.concat([e, el])) === false) e.preventDefault();
  }
  document.addEventListener('click', (e) => run('data-click', e));
  document.addEventListener('submit', (e) => run('data-submit', e));
  document.addEventListener('change', (e) => run('data-change', e));
})();
"#;

fn shell_open(title: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <link rel=\"icon\" href=\"data:,\">\n\
         <title>{} · tpt-homestack</title>\n<style>{CSS}{}</style>\n</head>\n<body>",
        esc(title),
        components::COMPONENTS_CSS
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
        for (path, label) in [
            ("/admin/users", "Admin · Users"),
            ("/admin/jobs", "Admin · Jobs"),
            ("/admin/status", "Admin · Status"),
        ] {
            let class = if active == path {
                " class=\"active\""
            } else {
                ""
            };
            links.push_str(&format!("<a href=\"{path}\"{class}>{label}</a>\n"));
        }
    }
    links.push_str("</nav>");
    format!(
        "<aside class=\"sidebar\">\n<div class=\"brand\">tpt-homestack</div>\n{links}\n\
         <div class=\"who\">signed in as <strong>{}</strong> <span class=\"badge\">{}</span><br>\
         <a href=\"/settings\">Settings</a><a href=\"#\" data-click=\"logout\">Sign out</a></div>\n</aside>",
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
    let toasts = components::toast_region();
    format!(
        "{}\n<div class=\"layout\">\n{}\n<main class=\"main\">\n<h1>{}</h1>\n{}\n</main>\n</div>\n{toasts}\n<script>\n\
         async function postJson(url, body) {{\n  const r = await fetch(url, {{method:'POST', headers:{{'Content-Type':'application/json'}}, body: JSON.stringify(body)}});\n  const j = await r.json().catch(() => ({{}}));\n  if (!r.ok) throw new Error(j.error || r.statusText);\n  return j;\n}}\n\
         function logout() {{ postJson('/api/auth/logout', {{}}).finally(() => location.href = '/login'); return false; }}\n{DISPATCH_JS}{}</script>\n</body>\n</html>",
        shell_open(title),
        sidebar(nav, active, user),
        esc(title),
        content,
        components::COMPONENTS_JS
    )
}

/// Standalone (no-shell) page used for login and first-run setup.
pub fn bare_page(title: &str, body: &str) -> String {
    format!(
        "{}\n<div class=\"center\">\n<div class=\"brand\" style=\"font-weight:700;font-size:20px;margin-bottom:16px\">tpt-homestack</div>\n{}\n</div>\n<script>{DISPATCH_JS}</script>\n</body>\n</html>",
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
         <form data-submit=\"doLogin\">\n\
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
         <form data-submit=\"doSetup\">\n\
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
    // A plain literal, not a format string: braces here are real JavaScript.
    // The script must use only what this page defines (`fetch`), and it shows
    // server messages as text, not HTML.
    r#"<div class="card" style="max-width:420px">
<h2 style="margin-top:0">Change password</h2>
<form data-submit="doPwChange">
<label for="cur">Current password</label><input id="cur" type="password" autocomplete="current-password" required>
<label for="new">New password (min 8 chars)</label><input id="new" type="password" autocomplete="new-password" required>
<div id="pwmsg"></div>
<div style="margin-top:14px"><button type="submit">Update password</button></div>
</form>
</div>
<script>
async function doPwChange(ev) {
  ev.preventDefault();
  const msg = document.getElementById('pwmsg');
  msg.replaceChildren();
  const box = document.createElement('div');
  try {
    const r = await fetch('/api/auth/change-password', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify({current: document.getElementById('cur').value, new_password: document.getElementById('new').value}),
    });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(j.error || r.statusText);
    box.className = 'msg ok';
    box.textContent = 'Password changed. Sign in again…';
    msg.appendChild(box);
    setTimeout(() => location.href = '/login', 1200);
  } catch (e) {
    box.className = 'msg error';
    box.textContent = e.message;
    msg.appendChild(box);
  }
  return false;
}
</script>"#
        .to_string()
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn settings_script_is_plain_javascript() {
        let html = settings_content();
        // A format-string leftover would leave doubled braces in the script.
        assert!(!html.contains("{{") && !html.contains("}}"), "{html}");
        // The script must not call helpers this page does not define.
        assert!(!html.contains("postJson"), "{html}");
        assert!(html.contains("fetch('/api/auth/change-password',"));
    }
}

/// Admin · users page.
pub fn users_admin_content(users: &[UserRow]) -> String {
    let pw_modal = components::modal(
        "pw-modal",
        "Reset password",
        &format!(
            "{}\n{}",
            components::field("New password (8+ characters)", "pw-new", "password", ""),
            components::button("Set password", "savePw", components::ButtonKind::Primary)
        ),
    );
    let rows: String = users
        .iter()
        .map(|u| {
            let status = if u.disabled {
                "<span class=\"badge warn\">disabled</span>".to_string()
            } else {
                "<span class=\"badge\">active</span>".to_string()
            };
            let role_sel = format!(
                "<select data-change=\"setRole:{}\">{}{} </select>",
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
                 <td class=\"row\">{}<button class=\"secondary\" data-click=\"resetPw:{}\">Reset pw</button>\
                 <button class=\"danger\" data-click=\"delUser:{}\">Delete</button></td></tr>",
                u.id,
                esc(&u.username),
                status,
                role_sel,
                if u.disabled {
                    format!("<button class=\"secondary\" data-click=\"setDisabled:{}:false\">Enable</button>", u.id)
                } else {
                    format!("<button class=\"secondary\" data-click=\"setDisabled:{}:true\">Disable</button>", u.id)
                },
                u.id,
                u.id
            )
        })
        .collect();
    format!(
        "<div class=\"card\">\n<h2 style=\"margin-top:0\">Create user</h2>\n\
         <form data-submit=\"doCreate\">\n\
         <div class=\"row\"><input id=\"nu\" placeholder=\"username\" style=\"max-width:200px\" required>\n\
         <input id=\"np\" type=\"password\" placeholder=\"password\" style=\"max-width:200px\" required>\n\
         <select id=\"nr\"><option value=\"user\">user</option><option value=\"admin\">admin</option></select>\n\
         <button type=\"submit\">Create</button></div>\n</form>\n</div>\n\
         <div class=\"card\">\n<table>\n<thead><tr><th>#</th><th>User</th><th>Status</th><th>Role</th><th>Actions</th></tr></thead>\n\
         <tbody>{rows}</tbody>\n</table>\n</div>\n{pw_modal}\n\
         <script>\nasync function api(url, method, body) {{\n  const r = await fetch(url, {{method, headers:{{'Content-Type':'application/json'}}, body: body ? JSON.stringify(body) : undefined}});\n  const j = await r.json().catch(() => ({{}}));\n  if (!r.ok) throw new Error(j.error || r.statusText);\n  return j;\n}}\n\
         function reload() {{ location.reload(); }}\nasync function doCreate(ev) {{ ev.preventDefault(); try {{ await api('/api/admin/users', 'POST', {{username: nu.value, password: np.value, role: nr.value}}); reload(); }} catch (e) {{ toast(e.message, false); }} return false; }}\n\
         async function setRole(id, ev, el) {{ try {{ await api('/api/admin/users/' + id, 'PATCH', {{role: el.value}}); reload(); }} catch (e) {{ toast(e.message, false); }} }}\n\
         async function setDisabled(id, flag) {{ try {{ await api('/api/admin/users/' + id, 'PATCH', {{disabled: flag === 'true'}}); reload(); }} catch (e) {{ toast(e.message, false); }} }}\n\
         let pwTarget = null;
async function resetPw(id) {{ pwTarget = id; document.getElementById('pw-new').value = ''; openModal('pw-modal'); }}
async function savePw() {{ const pw = document.getElementById('pw-new').value; if (pw.length < 8) {{ toast('password must be at least 8 characters', false); return; }} try {{ await api('/api/admin/users/' + pwTarget, 'PATCH', {{password: pw}}); closeModal('pw-modal'); toast('password updated', true); }} catch (e) {{ toast(e.message, false); }} }}
         async function delUser(id) {{ if (!confirm('Delete user ' + id + '?')) return; try {{ await api('/api/admin/users/' + id, 'DELETE'); reload(); }} catch (e) {{ toast(e.message, false); }} }}\n</script>"
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
         <form data-submit=\"doSubmit\">\n{inputs}\n\
         <div style=\"margin-top:14px\"><button type=\"submit\">Submit</button></div>\n</form>\n\
         <div id=\"pmsg\"></div>\n</div>\n\
         <script>\nasync function doSubmit(ev) {{\n  ev.preventDefault();\n  const body = {{}};\n  for (const f of [{}]) body[f] = document.getElementById('pf_' + f).value;\n  const r = await fetch('{action}', {{method:'POST', headers:{{'Content-Type':'application/json'}}, body: JSON.stringify(body)}});\n  const j = await r.json().catch(() => ({{}}));\n  const msg = document.getElementById('pmsg');\n  msg.innerHTML = r.ok ? '<div class=\"msg ok\">saved</div>' : '<div class=\"msg error\">' + (j.error || r.statusText) + '</div>';\n  return false;\n}}\n</script>",
        field_names.join(", ")
    )
}

/// Plugin error page fragment.
/// A `summary` page: one card per `(label, value)`, in a responsive grid.
/// One card on a plugin summary page.
pub enum MetricCard {
    /// A single value, shown large.
    Value { label: String, value: String },
    /// One row per group (`group`, `value`), shown as a small table.
    Rows {
        label: String,
        rows: Vec<(String, String)>,
    },
}

pub fn plugin_summary_page(metrics: &[MetricCard]) -> String {
    let cards: String = metrics
        .iter()
        .map(|metric| match metric {
            MetricCard::Value { label, value } => format!(
                "<div class=\"card\"><div class=\"muted\">{}</div>\
                 <div style=\"font-size:28px;font-weight:600;margin-top:4px\">{}</div></div>",
                esc(label),
                esc(value)
            ),
            MetricCard::Rows { label, rows } => {
                let body: String = if rows.is_empty() {
                    "<tr><td class=\"muted\" colspan=\"2\">Nothing yet.</td></tr>".to_string()
                } else {
                    rows.iter()
                        .map(|(group, value)| {
                            format!("<tr><td>{}</td><td>{}</td></tr>", esc(group), esc(value))
                        })
                        .collect()
                };
                format!(
                    "<div class=\"card\" style=\"grid-column:1/-1\"><div class=\"muted\">{}</div>\
                     <table style=\"margin-top:8px\"><tbody>{body}</tbody></table></div>",
                    esc(label)
                )
            }
        })
        .collect();
    format!(
        "<div style=\"display:grid;grid-template-columns:repeat(auto-fit,minmax(160px,1fr));gap:12px\">{cards}</div>"
    )
}

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

/// The first-run wizard at `/welcome`. `server_name` and the user list are
/// escaped here. The body is a plain string (its braces are JavaScript), with
/// `__NAME__` and `__USERS__` filled in.
pub fn welcome_content(server_name: &str, users: &[UserRow]) -> String {
    let list: String = users
        .iter()
        .map(|u| {
            format!(
                "<li>{} <span class=\"muted\">({})</span></li>",
                esc(&u.username),
                esc(&u.role)
            )
        })
        .collect();
    WELCOME_BODY
        .replace("__NAME__", &esc(server_name))
        .replace("__USERS__", &list)
}

const WELCOME_BODY: &str = r##"
<p class="muted">Five minutes to set this up. Each step can be changed later from the Admin pages.</p>
<div class="card">
  <h2 style="margin-top:0">1. Name this server</h2>
  <form>
    <div class="spread">
      <input id="sn" value="__NAME__" maxlength="64" required aria-label="Server name">
      <button type="submit">Save name</button>
    </div>
  </form>
  <div id="sn-msg"></div>
</div>
<div class="card">
  <h2 style="margin-top:0">2. Family accounts</h2>
  <p class="muted">Each person gets their own account and signs in with it.</p>
  <ul id="users">__USERS__</ul>
  <form>
    <div class="spread">
      <input id="nu" placeholder="username" required aria-label="Username">
      <input id="np" type="password" placeholder="password (8+ characters)" minlength="8" required aria-label="Password">
      <select id="nr" aria-label="Role"><option value="user">user</option><option value="admin">admin</option></select>
      <button type="submit">Add account</button>
    </div>
  </form>
  <div id="u-msg"></div>
</div>
<div class="card">
  <h2 style="margin-top:0">3. Connect from anywhere (optional)</h2>
  <p class="muted">Run <code>tpt-home remote status</code> on the server to see what is ready, then follow <code>docs/REMOTE_ACCESS.md</code> in the install folder. Nothing on this page changes remote access.</p>
</div>
<div class="card">
  <h2 style="margin-top:0">4. Apps</h2>
  <p class="muted">Apps will be added from an app catalog, which is not built yet. The apps on the Dashboard are the ones this server has now.</p>
  <button type="button" data-click="finish">Finish setup</button>
  <div id="f-msg"></div>
</div>
<script>
async function call(url, method, body) {
  const r = await fetch(url, {method, headers:{'Content-Type':'application/json'}, body: body ? JSON.stringify(body) : undefined});
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}
function note(id, text, ok) {
  document.getElementById(id).innerHTML = '<div class="msg ' + (ok ? 'ok' : 'error') + '"></div>';
  document.querySelector('#' + id + ' .msg').textContent = text;
}
async function saveName(ev) {
  ev.preventDefault();
  try {
    const s = await call('/api/admin/server', 'PUT', {name: document.getElementById('sn').value});
    note('sn-msg', 'Saved. This server is now called "' + s.name + '".', true);
  } catch (e) { note('sn-msg', e.message, false); }
  return false;
}
async function addUser(ev) {
  ev.preventDefault();
  try {
    const u = await call('/api/admin/users', 'POST', {
      username: document.getElementById('nu').value,
      password: document.getElementById('np').value,
      role: document.getElementById('nr').value
    });
    const li = document.createElement('li');
    li.textContent = u.username + ' (' + u.role + ')';
    document.getElementById('users').appendChild(li);
    document.getElementById('nu').value = '';
    document.getElementById('np').value = '';
    note('u-msg', 'Account added.', true);
  } catch (e) { note('u-msg', e.message, false); }
  return false;
}
async function finish() {
  try {
    await call('/api/admin/server', 'PUT', {wizard_done: true});
    location.href = '/dashboard';
  } catch (e) { note('f-msg', e.message, false); }
}
// Inline onsubmit attributes are blocked by the page's CSP, so forms are wired here.
document.getElementById('sn').form.addEventListener('submit', saveName);
document.getElementById('nu').form.addEventListener('submit', addUser);
</script>"##;

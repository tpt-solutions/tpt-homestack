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
        --line:#dde2e8; --accent:#2563eb; --accent-ink:#1d4ed8; --accent-soft:#dbeafe;
        --btn:#2563eb; --danger:#b91c1c; --ok:#15803d;
        --err-bg:#fee2e2; --ok-bg:#dcfce7; }
@media (prefers-color-scheme: dark) {
  :root { --bg:#12161c; --card:#1a212b; --text:#e6eaf0; --muted:#98a2ae;
          --line:#2a3340; --accent:#60a5fa; --accent-ink:#93c5fd; --accent-soft:#1e3a5f;
          --btn:#1d4ed8; --danger:#f87171; --ok:#4ade80;
          --err-bg:#3b1d22; --ok-bg:#17331f; }
}
* { box-sizing:border-box; }
/* A visible focus ring for keyboard users. Never remove it. */
:focus-visible { outline:3px solid var(--accent); outline-offset:2px; }
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
.sidebar nav a.active { background:var(--accent-soft); color:var(--accent-ink); font-weight:600; }
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
         background:var(--accent-soft); color:var(--accent-ink); }
.badge.warn { background:var(--err-bg); color:var(--danger); }
button, .btn { background:var(--btn); color:#fff; border:0; border-radius:8px;
        padding:8px 14px; font-size:14px; cursor:pointer; }
button.secondary { background:transparent; color:var(--accent); border:1px solid var(--accent); }
button.danger { background:transparent; color:var(--danger); border:1px solid var(--danger); }
td select { width:auto; min-width:120px; }
input, select, textarea { background:var(--bg); color:var(--text); border:1px solid var(--line);
        border-radius:8px; padding:8px 10px; font-size:14px; width:100%; }
label { display:block; font-size:13px; color:var(--muted); margin:10px 0 4px; }
table { border-collapse:collapse; width:100%; }
th, td { text-align:left; padding:8px 10px; border-bottom:1px solid var(--line); font-size:14px; }
th { color:var(--muted); font-weight:600; }
.center { max-width:380px; margin:12vh auto 0; }
.msg { padding:10px 12px; border-radius:8px; margin-bottom:12px; font-size:14px; }
.msg.error { background:var(--err-bg); color:var(--danger); }
.msg.ok { background:var(--ok-bg); color:var(--ok); }
.row { display:flex; gap:10px; align-items:center; flex-wrap:wrap; }
.spread { display:flex; justify-content:space-between; align-items:center; gap:10px; }
code, pre { font-family:ui-monospace,SFMono-Regular,Consolas,monospace; }
pre.md { background:var(--bg); border:1px solid var(--line); border-radius:8px;
         padding:14px; overflow-x:auto; }
#mobile-toggle { display:none; }
@media (max-width: 760px) {
  .layout { flex-direction:column; }
  .sidebar { width:auto; flex:none; border-right:0; border-bottom:1px solid var(--line); }
  .sidebar nav a { padding:12px 10px; }
  .main { padding:16px; }
  /* Touch targets: at least 44 px tall on phones (WCAG 2.5.5). */
  button, .btn, input, select, textarea { min-height:44px; }
}
/* Utility classes. Pages use these instead of style attributes, so the
   Content-Security-Policy can refuse inline styles. */
.mt0 { margin-top:0; } .m0 { margin:0; } .mt6 { margin-top:6px; }
.mt8 { margin-top:8px; } .mt14 { margin-top:14px; }
.scroll-x { overflow-x:auto; } .span-all { grid-column:1/-1; }
.w200 { max-width:200px; } .w420 { max-width:420px; } .w480 { max-width:480px; }
.between { justify-content:space-between; }
.form-grid { display:grid; gap:8px; max-width:520px; }
.brand-line { font-weight:700; font-size:20px; margin-bottom:16px; }
.big-number { font-size:28px; font-weight:600; margin-top:4px; }
.health-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(160px,1fr)); gap:14px; }
.cards-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(160px,1fr)); gap:12px; }
.label-block { display:block; margin:8px 0; }
.s-max-width-340px { max-width:340px; }
.s-display-none { display:none; }
.s-margin-top-12px { margin-top:12px; }
.s-max-width-280px { max-width:280px; }
.s-max-width-140px { max-width:140px; }
.s-margin-top-10px { margin-top:10px; }
.s-max-width-170px { max-width:170px; }
.s-gap-8px-flex-wrap-wrap { gap:8px;flex-wrap:wrap; }
.s-max-width-240px { max-width:240px; }
.s-max-width-160px { max-width:160px; }
.s-display-flex-flex-wrap-wrap-gap-16px { display:flex;flex-wrap:wrap;gap:16px; }
.s-margin-top-16px { margin-top:16px; }
.s-flex-1-1-220px-max-width-300px-margin-0 { flex:1 1 220px;max-width:300px;margin:0; }
.s-font-size-2em-font-weight-600 { font-size:2em;font-weight:600; }
.s-gap-24px { gap:24px; }
.s-max-width-220px { max-width:220px; }
.s-white-space-pre-wrap { white-space:pre-wrap; }
.s-white-space-pre-wrap-max-height-8em-overflow-auto { white-space:pre-wrap;max-height:8em;overflow:auto; }
.s-max-width-90px { max-width:90px; }
.s-max-width-110px { max-width:110px; }
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
            ("/admin/plugins", "Admin · Plugins"),
            ("/admin/remote", "Admin · Remote access"),
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
        "{}\n<div class=\"center\">\n<div class=\"brand brand-line\">tpt-homestack</div>\n{}\n</div>\n<script>{DISPATCH_JS}</script>\n</body>\n</html>",
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
        "<div class=\"card\">\n{err}\n<h1 class=\"mt0\">Sign in</h1>\n\
         <form data-submit=\"doLogin\">\n\
         <label for=\"u\">Username</label><input id=\"u\" autocomplete=\"username\" required>\n\
         <label for=\"p\">Password</label><input id=\"p\" type=\"password\" autocomplete=\"current-password\" required>\n\
         <div class=\"mt14\"><button type=\"submit\">Sign in</button></div>\n</form>\n</div>\n\
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
        "<div class=\"card\">\n{err}\n<h1 class=\"mt0\">Welcome</h1>\n\
         <p class=\"muted\">This instance has no users yet. Create the first admin account.</p>\n\
         <form data-submit=\"doSetup\">\n\
         <label for=\"u\">Admin username</label><input id=\"u\" autocomplete=\"username\" required>\n\
         <label for=\"p\">Password (min 8 chars)</label><input id=\"p\" type=\"password\" autocomplete=\"new-password\" required>\n\
         <div class=\"mt14\"><button type=\"submit\">Create admin</button></div>\n</form>\n</div>\n\
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
    let fields = format!(
        "{}
{}
<div id=\"pwmsg\"></div>
<div class=\"mt14\">{}</div>",
        components::field_with(
            "Current password",
            "cur",
            "password",
            "",
            "autocomplete=\"current-password\" required",
        ),
        components::field_with(
            "New password (min 8 chars)",
            "new",
            "password",
            "",
            "autocomplete=\"new-password\" required",
        ),
        components::submit("Update password"),
    );
    format!(
        "<div class=\"card w420\">
<h2 class=\"mt0\">Change password</h2>
{}
</div>
{SETTINGS_SCRIPT}",
        components::form("doPwChange", &fields)
    )
}

/// The settings page script. A plain literal, not a format string: braces here
/// are real JavaScript. It uses only what this page defines (`fetch`), and it
/// shows server messages as text, not HTML.
const SETTINGS_SCRIPT: &str = r#"<script>
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
</script>"#;

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
                "<select data-change=\"setRole:{}\" aria-label=\"Role\">{}{} </select>",
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
        "<div class=\"card\">\n<h2 class=\"mt0\">Create user</h2>\n\
         <form data-submit=\"doCreate\">\n\
         <div class=\"row\"><input id=\"nu\" placeholder=\"username\" aria-label=\"New username\" required class=\"w200\">\n\
         <input id=\"np\" type=\"password\" placeholder=\"password\" aria-label=\"New password\" required class=\"w200\">\n\
         <select id=\"nr\" aria-label=\"New user role\"><option value=\"user\">user</option><option value=\"admin\">admin</option></select>\n\
         <button type=\"submit\">Create</button></div>\n</form>\n</div>\n\
         <div class=\"card scroll-x\">\n<table>\n<thead><tr><th>#</th><th>User</th><th>Status</th><th>Role</th><th>Actions</th></tr></thead>\n\
         <tbody>{rows}</tbody>\n</table>\n</div>\n{pw_modal}\n\
         <script>\nasync function api(url, method, body) {{\n  const r = await fetch(url, {{method, headers:{{'Content-Type':'application/json'}}, body: body ? JSON.stringify(body) : undefined}});\n  const j = await r.json().catch(() => ({{}}));\n  if (!r.ok) throw new Error(j.error || r.statusText);\n  return j;\n}}\n\
         function reload() {{ location.reload(); }}\nasync function doCreate(ev) {{ ev.preventDefault(); try {{ await api('/api/admin/users', 'POST', {{username: nu.value, password: np.value, role: nr.value}}); reload(); }} catch (e) {{ toast(e.message, false); }} return false; }}\n\
         async function setRole(id, ev, el) {{ try {{ await api('/api/admin/users/' + id, 'PATCH', {{role: el.value}}); reload(); }} catch (e) {{ toast(e.message, false); }} }}\n\
         async function setDisabled(id, flag) {{ try {{ await api('/api/admin/users/' + id, 'PATCH', {{disabled: flag === 'true'}}); reload(); }} catch (e) {{ toast(e.message, false); }} }}\n\
         let pwTarget = null;
async function resetPw(id) {{ pwTarget = id; document.getElementById('pw-new').value = ''; openModal('pw-modal'); }}
async function savePw() {{ const pw = document.getElementById('pw-new').value; if (pw.length < 8) {{ toast('password must be at least 8 characters', false); return; }} try {{ await api('/api/admin/users/' + pwTarget, 'PATCH', {{password: pw}}); closeModal('pw-modal'); toast('password updated', true); }} catch (e) {{ toast(e.message, false); }} }}
         async function delUser(id) {{ if (!(await confirmDialog('Delete user ' + id + '?', 'Delete'))) return; try {{ await api('/api/admin/users/' + id, 'DELETE'); reload(); }} catch (e) {{ toast(e.message, false); }} }}\n</script>"
    )
}

/// Capability keys a plugin can request and an admin can approve, in the
/// order the approval form shows them.
pub const CAPABILITY_KEYS: [&str; 5] = ["db", "kv", "storage", "http", "notify"];

/// What approving a capability lets the plugin do, in plain words. Unknown
/// keys get a generic label so a new capability is never shown as nothing.
pub fn capability_label(key: &str) -> &'static str {
    match key {
        "db" => "Keep its own records in the database (only its own tables)",
        "kv" => "Save small settings and counters",
        "storage" => "Store and read files in its own folder",
        "http" => "Make web requests to the hosts it names",
        "notify" => "Send messages to the notification channels you set up",
        _ => "Use an unknown capability",
    }
}

/// Admin · plugins page: install a package with plain-language approvals,
/// then enable, disable or remove installed plugins. The table is filled from
/// `GET /api/plugins` by the script, so the page itself holds no plugin data.
pub fn plugins_admin_content() -> String {
    let boxes: String = CAPABILITY_KEYS
        .iter()
        .map(|key| {
            format!(
                "<label class=\"check\"><input type=\"checkbox\" name=\"cap\" value=\"{key}\" checked> {}</label>\n",
                esc(capability_label(key))
            )
        })
        .collect();
    let labels: serde_json::Map<String, serde_json::Value> = CAPABILITY_KEYS
        .iter()
        .map(|key| {
            (
                key.to_string(),
                serde_json::Value::from(capability_label(key)),
            )
        })
        .collect();
    let labels = serde_json::Value::Object(labels);
    format!(
        "<div class=\"card\">\n<h2 class=\"mt0\">Install a plugin</h2>\n\
         <form data-submit=\"installPlugin\">\n\
         <p>Choose a <code>.tptp</code> package. Only the ticked abilities are granted; the plugin is refused anything else.</p>\n\
         <input id=\"pkg\" type=\"file\" accept=\".tptp\" aria-label=\"Plugin package (.tptp)\" required>\n\
         <p class=\"muted\">If the publisher sent a signature file (<code>.sig</code>), choose it too. A signed package installs only when its key is trusted by this server.</p>\n\
         <input id=\"sig\" type=\"file\" accept=\".sig,text/plain\" aria-label=\"Signature file (optional)\">\n\
         <fieldset><legend>Allow this plugin to</legend>\n{boxes}</fieldset>\n\
         <label class=\"check\"><input type=\"checkbox\" id=\"pen\" checked> Enable after install</label>\n\
         <button type=\"submit\">Install</button>\n</form>\n</div>\n\
         <div class=\"card scroll-x\">\n<table>\n<thead><tr><th>Plugin</th><th>Status</th><th>Allowed to</th><th>Actions</th></tr></thead>\n\
         <tbody id=\"plugins\"><tr><td colspan=\"4\">Loading…</td></tr></tbody>\n</table>\n</div>\n\
         <div class=\"card\">\n<div class=\"row between\"><h2 class=\"m0\">Plugin catalog</h2>\
         <button class=\"secondary\" data-click=\"loadCatalog\">Load catalog</button></div>\n\
         <p class=\"muted\">Plugins listed by the catalog address in config.toml. Each one is downloaded, checksum-checked and signature-checked before you approve it.</p>\n\
         <div id=\"catalog\"></div>\n</div>\n\
         <script>\nconst CAP_LABELS = {labels};\n\
         let catalogPlugins = [];\n\
         async function loadCatalog() {{\n\
           const box = document.getElementById('catalog');\n\
           box.innerHTML = '<p class=\"muted\">Loading…</p>';\n\
           try {{\n\
             const c = await api('/api/plugins/catalog', 'GET');\n\
             if (!c.configured) {{ box.innerHTML = '<p class=\"muted\">No catalog is set. Set catalog_url in config.toml to browse one.</p>'; return false; }}\n\
             catalogPlugins = c.plugins;\n\
             if (!c.plugins.length) {{ box.innerHTML = '<p class=\"muted\">The catalog lists no plugins.</p>'; return false; }}\n\
             box.innerHTML = c.plugins.map((p) =>\n\
               '<div class=\"card\"><strong>' + escHtml(p.name) + '</strong> <code>' + escHtml(p.id) + '</code> ' + escHtml(p.version) +\n\
               ' <span class=\"badge\">' + escHtml(p.status) + '</span>' +\n\
               '<p>' + escHtml(p.description) + '</p><ul>' + p.permissions_plain.map((s) => '<li>' + escHtml(s) + '</li>').join('') + '</ul>' +\n\
               '<p class=\"muted\">' + (p.signed ? 'Signed' + (p.signed_by ? ' (key ' + escHtml(p.signed_by.slice(0, 12)) + '…)' : '') : 'Not signed') + '.</p>' +\n\
               '<button data-click=\"installCatalog:' + escHtml(p.id) + '\">Install…</button></div>'\n\
             ).join('');\n\
           }} catch (e) {{ box.textContent = e.message; }}\n\
           return false;\n\
         }}\n\
         async function installCatalog(id) {{\n\
           const p = catalogPlugins.find((x) => x.id === id);\n\
           if (!p) {{ toast('reload the catalog first', false); return; }}\n\
           const abilities = p.permissions_plain.length ? p.permissions_plain.map((s) => '- ' + s).join('\\n') : '- Nothing';\n\
           if (!(await confirmDialog('Install ' + p.name + ' ' + p.version + '?\\n\\nIt will be allowed to:\\n' + abilities, 'Install', false))) return;\n\
           try {{\n\
             await api('/api/plugins/catalog/install', 'POST', {{id: p.id, approve: p.permissions, enable: true}});\n\
             toast('installed ' + p.id, true);\n\
             loadCatalog();\n\
             loadPlugins();\n\
           }} catch (e) {{ toast(e.message, false); }}\n\
         }}\n\
         const escHtml = (s) => String(s).replace(/[&<>\"']/g, (c) => ({{'&':'&amp;','<':'&lt;','>':'&gt;','\"':'&quot;',\"'\":'&#39;'}})[c]);\n\
         const asList = (v) => (typeof v === 'string' ? JSON.parse(v) : (v || []));\n\
         async function api(url, method, body) {{\n  const r = await fetch(url, {{method, headers:{{'Content-Type':'application/json'}}, body: body ? JSON.stringify(body) : undefined}});\n  const j = await r.json().catch(() => ({{}}));\n  if (!r.ok) throw new Error(j.error || r.statusText);\n  return j;\n}}\n\
         async function loadPlugins() {{\n\
           const tb = document.getElementById('plugins');\n\
           try {{\n\
             const list = await api('/api/plugins', 'GET');\n\
             if (!list.length) {{ tb.innerHTML = '<tr><td colspan=\"4\">No plugins installed yet.</td></tr>'; return; }}\n\
             tb.innerHTML = list.map((p) => {{\n\
               const id = escHtml(p.id);\n\
               const approved = asList(p.approved_permissions);\n\
               const requested = asList(p.permissions);\n\
               const allowed = approved.length ? approved.map((k) => '<li>' + escHtml(CAP_LABELS[k] || k) + '</li>').join('') : '<li>Nothing</li>';\n\
               const denied = requested.filter((k) => !approved.includes(k)).map((k) => '<li class=\"warn\">Not allowed: ' + escHtml(CAP_LABELS[k] || k) + '</li>').join('');\n\
               const status = p.enabled ? '<span class=\"badge\">enabled</span>' : '<span class=\"badge warn\">disabled</span>';\n\
               const toggle = p.enabled\n\
                 ? '<button class=\"secondary\" data-click=\"togglePlugin:' + id + ':false\">Disable</button>'\n\
                 : '<button class=\"secondary\" data-click=\"togglePlugin:' + id + ':true\">Enable</button>';\n\
               return '<tr><td><strong>' + escHtml(p.name) + '</strong><br><code>' + id + '</code> ' + escHtml(p.version) + '</td>' +\n\
                 '<td>' + status + '</td><td><ul>' + allowed + denied + '</ul></td>' +\n\
                 '<td class=\"row\">' + toggle +\n\
                 '<button class=\"secondary\" data-click=\"removePlugin:' + id + ':true\">Remove, keep data</button>' +\n\
                 '<button class=\"danger\" data-click=\"removePlugin:' + id + ':false\">Remove and delete data</button></td></tr>';\n\
             }}).join('');\n\
           }} catch (e) {{ tb.innerHTML = '<tr><td colspan=\"4\">' + escHtml(e.message) + '</td></tr>'; }}\n\
         }}\n\
         async function installPlugin(ev) {{\n\
           ev.preventDefault();\n\
           const file = document.getElementById('pkg').files[0];\n\
           if (!file) {{ toast('choose a package first', false); return false; }}\n\
           const approved = [...document.querySelectorAll('input[name=cap]:checked')].map((c) => c.value);\n\
           const form = new FormData();\n\
           form.append('package', file);\n\
           form.append('approve', approved.join(','));\n\
           form.append('enable', document.getElementById('pen').checked ? 'true' : 'false');\n\
           const sigFile = document.getElementById('sig').files[0];\n\
           if (sigFile) form.append('signature', await sigFile.text());\n\
           try {{\n\
             const r = await fetch('/api/plugins/install', {{method: 'POST', body: form, headers: {{'X-TPT-Request': '1'}}}});\n\
             const j = await r.json().catch(() => ({{}}));\n\
             if (!r.ok) throw new Error(j.error || r.statusText);\n\
             toast('installed ' + j.id, true);\n\
             loadPlugins();\n\
           }} catch (e) {{ toast(e.message, false); }}\n\
           return false;\n\
         }}\n\
         async function togglePlugin(id, flag) {{\n\
           try {{ await api('/api/plugins/' + encodeURIComponent(id) + (flag === 'true' ? '/enable' : '/disable'), 'POST'); loadPlugins(); }} catch (e) {{ toast(e.message, false); }}\n\
         }}\n\
         async function removePlugin(id, keep) {{\n\
           const msg = keep === 'true' ? 'Remove ' + id + ' and keep its data?' : 'Remove ' + id + ' and delete all its data? This cannot be undone.';\n\
           if (!(await confirmDialog(msg, 'Remove'))) return;\n\
           try {{ await api('/api/plugins/' + encodeURIComponent(id) + '?keep_data=' + keep, 'DELETE'); toast('removed ' + id, true); loadPlugins(); }} catch (e) {{ toast(e.message, false); }}\n\
         }}\n\
         loadPlugins();\n</script>"
    )
}

/// Admin · Remote access page: live checks of Tailscale, cloudflared and
/// Netbird and the steps still open. The script fills it from
/// `GET /api/admin/remote`; config changes are shown as commands to run on the
/// server, since they ask for confirmation there.
pub fn remote_admin_content() -> String {
    "<div class=\"card\">\n<div class=\"row between\">\
     <h2 class=\"m0\">Connect from anywhere</h2>\
     <button class=\"secondary\" data-click=\"loadRemote\">Check again</button></div>\n\
     <p class=\"muted\">These checks only look; nothing on this computer changes until you run a command below on the server.</p>\n\
     <div id=\"remote-config\"></div>\n</div>\n\
     <div class=\"card\"><h2 class=\"mt0\">Steps</h2><div id=\"remote-steps\">Checking…</div></div>\n\
     <script>\n\
     const escRemote = (s) => String(s).replace(/[&<>\"']/g, (c) => ({'&':'&amp;','<':'&lt;','>':'&gt;','\"':'&quot;',\"'\":'&#39;'})[c]);\n\
     const STAGE_LABEL = {done: 'Done', todo: 'To do', blocked: 'Install first'};\n\
     async function loadRemote() {\n\
       const cfgBox = document.getElementById('remote-config');\n\
       const stepBox = document.getElementById('remote-steps');\n\
       try {\n\
         const r = await fetch('/api/admin/remote');\n\
         const j = await r.json().catch(() => ({}));\n\
         if (!r.ok) throw new Error(j.error || r.statusText);\n\
         const c = j.config;\n\
         cfgBox.innerHTML = '<p>Server: <code>' + escRemote(c.host) + ':' + escRemote(c.port) + '</code>, secure cookies ' +\n\
           (c.secure_cookies ? 'on' : 'off') + ', trust proxy ' + (c.trust_proxy ? 'on' : 'off') + '.</p>' +\n\
           (c.exposed ? '<p class=\"badge warn\">Exposed to the network</p>' : '') +\n\
           '<p>Tailscale: ' + escRemote(j.tailscale) + '</p>' +\n\
           (j.probe ? '<p>' + (j.probe.reachable ? '<span class=\"badge\">Answers</span> ' : '<span class=\"badge warn\">Not answering</span> ') +\n\
             '<code>' + escRemote(j.probe.url) + '</code>' + (j.probe.error ? ' — ' + escRemote(j.probe.error) : '') + '</p>' : '');\n\
         stepBox.innerHTML = '<ol>' + j.steps.map((s) =>\n\
           '<li><span class=\"badge' + (s.stage === 'done' ? '' : (s.stage === 'blocked' ? ' warn' : '')) + '\">' +\n\
           STAGE_LABEL[s.stage] + '</span> <strong>' + escRemote(s.provider) + '</strong>: ' + escRemote(s.text) +\n\
           (s.command ? '<br><code>' + escRemote(s.command) + '</code>' : '') + '</li>'\n\
         ).join('') + '</ol>';\n\
       } catch (e) {\n\
         stepBox.textContent = e.message;\n\
       }\n\
       return false;\n\
     }\n\
     loadRemote();\n\
     </script>"
        .to_string()
}

/// `text` as a QR code in inline SVG, or `None` if it cannot be encoded.
pub fn qr_svg(text: &str) -> Option<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    Some(
        code.render::<qrcode::render::svg::Color>()
            .min_dimensions(200, 200)
            .quiet_zone(true)
            .build(),
    )
}

/// Settings card showing the server's network address for phones and tablets,
/// with a QR code of it when one can be made.
pub fn phone_link_card(url: &str, qr: Option<&str>) -> String {
    let code = qr
        .map(|svg| format!("<div class=\"qr\">{svg}</div>\n"))
        .unwrap_or_default();
    format!(
        "<div class=\"card\">\n<h2 class=\"mt0\">Open on a phone or tablet</h2>\n\
         <p>On the same Wi-Fi or network, open <a href=\"{url}\"><code>{url}</code></a>, or scan the code.</p>\n\
         {code}<p class=\"muted\">The name works where the device can resolve <code>.local</code> names: Windows 10 or later, Apple devices and most Linux desktops. Some Android phones need the server's IP address and port instead.</p>\n</div>",
        url = esc(url),
    )
}

/// Settings card for a server that listens only on this computer.
pub fn phone_link_unavailable_card() -> String {
    "<div class=\"card\">\n<h2 class=\"mt0\">Open on a phone or tablet</h2>\n\
     <p>This server only accepts connections from this computer. To use it from other devices, set <code>host</code> to <code>0.0.0.0</code> in <code>config.toml</code> (or <code>TPT_HOME_HOST</code>) and restart.</p>\n</div>"
        .to_string()
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
        "<div class=\"card scroll-x\">\n<table>\n<thead><tr>{headers}</tr></thead>\n<tbody>{body}</tbody>\n</table>\n\
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
        .map(|f| components::field(f, &format!("pf_{f}"), "text", ""))
        .collect::<Vec<_>>()
        .join("\n");
    let body = format!(
        "{inputs}\n<div class=\"mt14\">{}</div>",
        components::submit("Submit")
    );
    let field_names: Vec<String> = fields.iter().map(|f| format!("'{f}'")).collect();
    format!(
        "<div class=\"card w480\">\n{}\n\
         <div id=\"pmsg\"></div>\n</div>\n\
         <script>\nasync function doSubmit(ev) {{\n  ev.preventDefault();\n  const body = {{}};\n  for (const f of [{}]) body[f] = document.getElementById('pf_' + f).value;\n  const r = await fetch('{action}', {{method:'POST', headers:{{'Content-Type':'application/json'}}, body: JSON.stringify(body)}});\n  const j = await r.json().catch(() => ({{}}));\n  const msg = document.getElementById('pmsg');\n  msg.innerHTML = r.ok ? '<div class=\"msg ok\">saved</div>' : '<div class=\"msg error\">' + (j.error || r.statusText) + '</div>';\n  return false;\n}}\n</script>",
        components::form("doSubmit", &body),
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
                 <div class=\"big-number\">{}</div></div>",
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
                    "<div class=\"card span-all\"><div class=\"muted\">{}</div>\
                     <table class=\"mt8\"><tbody>{body}</tbody></table></div>",
                    esc(label)
                )
            }
        })
        .collect();
    format!("<div class=\"cards-grid\">{cards}</div>")
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
pub fn welcome_content(server_name: &str, users: &[UserRow], phone_html: &str) -> String {
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
        .replace("__PHONE__", phone_html)
}

const WELCOME_BODY: &str = r##"
<p class="muted">Five minutes to set this up. Each step can be changed later from the Admin pages.</p>
<div class="card">
  <h2 class="mt0">1. Name this server</h2>
  <form>
    <div class="spread">
      <input id="sn" value="__NAME__" maxlength="64" required aria-label="Server name">
      <button type="submit">Save name</button>
    </div>
  </form>
  <div id="sn-msg"></div>
</div>
<div class="card">
  <h2 class="mt0">2. Family accounts</h2>
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
  __PHONE__
</div>
<div class="card">
  <h2 class="mt0">3. Connect from anywhere (optional)</h2>
  <p class="muted"><a href="/admin/remote">Remote access</a> checks Tailscale, Cloudflare and Netbird on this computer and lists the steps still to do. Changing the settings is done with <code>tpt-home remote setup</code> on the server, which asks before it changes anything.</p>
</div>
<div class="card">
  <h2 class="mt0">4. Apps</h2>
  <p class="muted">If the server has a catalog address set (<code>catalog_url</code> in config.toml), <a href="/admin/plugins">Admin · Plugins</a> lists the apps it offers, and you approve each one before it is installed. The apps on the Dashboard are the ones this server has now.</p>
  <div id="starter"><button type="button" class="secondary" data-click="loadStarter">Show starter apps</button></div>
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
let starterPlugins = [];
const escW = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'})[c]);
async function loadStarter() {
  const box = document.getElementById('starter');
  try {
    const c = await call('/api/plugins/catalog', 'GET');
    if (!c.configured) { box.innerHTML = '<p class="muted">No app catalog is set on this server (catalog_url in config.toml).</p>'; return false; }
    starterPlugins = c.plugins.filter((p) => p.status === 'not installed');
    if (!starterPlugins.length) { box.innerHTML = '<p class="muted">Every app in the catalog is already installed.</p>'; return false; }
    box.innerHTML = starterPlugins.map((p, i) =>
      '<label class="label-block"><input type="checkbox" id="st-' + i + '"> <strong>' + escW(p.name) + '</strong> ' + escW(p.description) +
      '<br><span class="muted">Allowed to: ' + escW(p.permissions_plain.join('; ') || 'nothing') + '</span></label>'
    ).join('') + '<button type="button" data-click="installStarter">Install selected</button><div id="st-msg"></div>';
  } catch (e) { box.textContent = e.message; }
  return false;
}
async function installStarter() {
  const picked = starterPlugins.filter((p, i) => document.getElementById('st-' + i)?.checked);
  if (!picked.length) { note('st-msg', 'Choose at least one app.', false); return false; }
  let done = 0;
  for (const p of picked) {
    try {
      await call('/api/plugins/catalog/install', 'POST', {id: p.id, approve: p.permissions, enable: true});
      done++;
    } catch (e) { note('st-msg', p.name + ': ' + e.message, false); }
  }
  note('st-msg', 'Installed ' + done + ' of ' + picked.length + '.', done === picked.length);
  return false;
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

#[cfg(test)]
mod capability_label_tests {
    use super::*;

    #[test]
    fn every_capability_has_a_distinct_plain_label() {
        let labels: Vec<&str> = CAPABILITY_KEYS
            .iter()
            .map(|k| capability_label(k))
            .collect();
        for (i, label) in labels.iter().enumerate() {
            assert!(
                !label.starts_with("Use an unknown"),
                "{}",
                CAPABILITY_KEYS[i]
            );
            assert!(!label.contains('_'), "no raw permission names: {label}");
            for other in &labels[i + 1..] {
                assert_ne!(label, other);
            }
        }
    }

    #[test]
    fn unknown_capability_gets_a_generic_label() {
        assert_eq!(capability_label("teleport"), "Use an unknown capability");
    }
}

/// Colour contrast for the stylesheet's text on background pairs, read from the
/// tokens in [`CSS`] so a change to a colour is checked against WCAG AA.
#[cfg(test)]
mod contrast_tests {
    use super::CSS;
    use std::collections::HashMap;

    /// `--name:#rrggbb` declarations in `block`.
    fn tokens(block: &str) -> HashMap<String, [u8; 3]> {
        let mut out = HashMap::new();
        for part in block.split("--").skip(1) {
            let Some((name, rest)) = part.split_once(':') else {
                continue;
            };
            let name = name.trim();
            let hex = rest.trim_start();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
                continue;
            }
            if let Some(hex) = hex.strip_prefix('#').and_then(|h| h.get(..6)) {
                let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
                if let (Some(r), Some(g), Some(b)) = (channel(0), channel(2), channel(4)) {
                    out.insert(name.to_string(), [r, g, b]);
                }
            }
        }
        out
    }

    fn luminance([r, g, b]: [u8; 3]) -> f64 {
        let lin = |c: u8| {
            let v = f64::from(c) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    fn ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// The light palette, then the dark one, from the stylesheet.
    fn palettes() -> [(&'static str, HashMap<String, [u8; 3]>); 2] {
        let dark_at = CSS
            .find("@media (prefers-color-scheme: dark)")
            .expect("dark palette block");
        let light = tokens(&CSS[..dark_at]);
        let dark = tokens(&CSS[dark_at..]);
        [("light", light), ("dark", dark)]
    }

    /// Text colour, background colour pairs the pages actually use.
    const PAIRS: &[(&str, &str)] = &[
        ("text", "bg"),
        ("text", "card"),
        ("muted", "bg"),
        ("muted", "card"),
        ("accent", "bg"),
        ("accent", "card"),
        ("accent-ink", "accent-soft"),
        ("danger", "card"),
        ("danger", "err-bg"),
        ("ok", "card"),
        ("ok", "ok-bg"),
    ];

    #[test]
    fn text_on_every_surface_meets_wcag_aa() {
        for (theme, colours) in palettes() {
            for (fg, bg) in PAIRS {
                let f = colours
                    .get(*fg)
                    .unwrap_or_else(|| panic!("{theme}: --{fg}"));
                let b = colours
                    .get(*bg)
                    .unwrap_or_else(|| panic!("{theme}: --{bg}"));
                let r = ratio(*f, *b);
                assert!(
                    r >= 4.5,
                    "{theme}: --{fg} on --{bg} is {r:.2}:1, needs 4.5:1"
                );
            }
        }
    }

    #[test]
    fn white_label_on_the_primary_button_meets_wcag_aa() {
        for (theme, colours) in palettes() {
            let btn = colours
                .get("btn")
                .unwrap_or_else(|| panic!("{theme}: --btn"));
            let r = ratio([255, 255, 255], *btn);
            assert!(r >= 4.5, "{theme}: white on --btn is {r:.2}:1");
        }
    }

    #[test]
    fn focus_rings_are_never_removed() {
        assert!(!CSS.contains("outline:none") && !CSS.contains("outline: none"));
        assert!(!CSS.contains("outline:0") && !CSS.contains("outline: 0"));
        assert!(CSS.contains(":focus-visible"));
    }

    #[test]
    fn the_check_reads_real_colours() {
        // Guards against the parser silently finding nothing.
        for (theme, colours) in palettes() {
            assert!(
                colours.len() >= 10,
                "{theme}: only {} tokens parsed",
                colours.len()
            );
        }
    }
}

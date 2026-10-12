// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Tiny safe markdown renderer (no external deps, no XSS surface).
//!
//! Pipeline: HTML-escape **first**, then apply a markdown subset on the
//! escaped text. The only thing that can become a link is `[text](scheme://…)`
//! with an `http`, `https` or `mailto` scheme — everything else stays text.

/// Render the supported markdown subset to sanitized HTML.
pub fn markdown_to_html(src: &str) -> String {
    use tpt_home_ui::esc;

    let mut out = String::with_capacity(src.len() + 64);
    let mut in_code = false;
    let mut in_list = false;

    for line in src.lines() {
        let trimmed = line.trim();

        // Fenced code blocks.
        if trimmed.starts_with("```") {
            if in_code {
                out.push_str("</code></pre>\n");
                in_code = false;
            } else {
                close_list(&mut out, &mut in_list);
                out.push_str("<pre class=\"md\"><code>");
                in_code = true;
            }
            continue;
        }
        if in_code {
            out.push_str(&esc(line));
            out.push('\n');
            continue;
        }

        // Headings.
        if let Some(rest) = trimmed.strip_prefix("### ") {
            close_list(&mut out, &mut in_list);
            out.push_str(&format!("<h3>{}</h3>\n", inline(rest)));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            close_list(&mut out, &mut in_list);
            out.push_str(&format!("<h2>{}</h2>\n", inline(rest)));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            close_list(&mut out, &mut in_list);
            out.push_str(&format!("<h1>{}</h1>\n", inline(rest)));
            continue;
        }

        // Lists.
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            if !in_list {
                out.push_str("<ul>\n");
                in_list = true;
            }
            out.push_str(&format!("<li>{}</li>\n", inline(rest)));
            continue;
        }
        close_list(&mut out, &mut in_list);

        if trimmed.is_empty() {
            continue;
        }
        out.push_str(&format!("<p>{}</p>\n", inline(trimmed)));
    }

    if in_code {
        out.push_str("</code></pre>\n");
    }
    close_list(&mut out, &mut in_list);
    out
}

fn close_list(out: &mut String, in_list: &mut bool) {
    if *in_list {
        out.push_str("</ul>\n");
        *in_list = false;
    }
}

/// Inline formatting: escape **first**, then apply markup on escaped text.
fn inline(raw: &str) -> String {
    let s = tpt_home_ui::esc(raw);

    // Links: [text](scheme://rest) — scheme restricted, text/URL already
    // escaped so no attribute breakout is possible.
    let s = render_links(&s);

    // `code`
    let s = toggle_delims(&s, '`', "<code>", "</code>");
    // **bold**
    toggle_delims(&s, '*', "<strong>", "</strong>")
}

fn render_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'['
            && let Some(close_bracket) = s[i + 1..].find(']')
        {
            let text_end = i + 1 + close_bracket;
            if s[text_end + 1..].starts_with('(')
                && let Some(close_paren) = s[text_end + 2..].find(')')
            {
                let url = &s[text_end + 2..text_end + 2 + close_paren];
                if let Some(safe_url) = safe_url(url) {
                    out.push_str(&format!(
                        "<a href=\"{}\" rel=\"noopener noreferrer\" target=\"_blank\">{}</a>",
                        safe_url,
                        &s[i + 1..text_end]
                    ));
                    i = text_end + 2 + close_paren + 1;
                    continue;
                }
            }
        }
        out.push_str(&s[i..i + 1]);
        i += 1;
    }
    out
}

fn safe_url(url: &str) -> Option<&str> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
    {
        // Reject quotes/backslashes so the (already escaped) href can't
        // re-introduce markup.
        if url.contains(['"', '\\', '<', '>']) {
            None
        } else {
            Some(url)
        }
    } else {
        None
    }
}

/// Pair-wise delimiter replacement (`**a**` → `<strong>a</strong>`), applied
/// to escaped text so no markup can slip through.
fn toggle_delims(s: &str, delim: char, open: &str, close: &str) -> String {
    // Bold uses a two-char marker; code uses single.
    let marker_len = if open == "<strong>" { 2 } else { 1 };
    let marker: String = std::iter::repeat_n(delim, marker_len).collect();
    let mut out = String::with_capacity(s.len());
    let mut is_open = true;
    let mut rest = s;
    while let Some(pos) = rest.find(&marker) {
        out.push_str(&rest[..pos]);
        out.push_str(if is_open { open } else { close });
        is_open = !is_open;
        rest = &rest[pos + marker.len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html_before_markdown() {
        let html = markdown_to_html("<script>alert(1)</script>");
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn headings_bold_code() {
        let html = markdown_to_html("# Title\n**bold** and `code`");
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<code>code</code>"));
    }

    #[test]
    fn only_safe_links_render() {
        let html = markdown_to_html(
            "[ok](https://example.com) [bad](javascript:alert(1)) [x](\"onclick=)",
        );
        assert!(html.contains("<a href=\"https://example.com\""));
        // javascript: and quote-breaking URLs must not become links — they
        // may survive as inert escaped text.
        assert_eq!(html.matches("<a ").count(), 1);
    }

    #[test]
    fn lists_and_code_blocks() {
        let html = markdown_to_html("- one\n- two\n```\nlet x = 1;\n```");
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>one</li>"));
        assert!(html.contains("<pre class=\"md\"><code>let x = 1;\n</code></pre>"));
    }
}

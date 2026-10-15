// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Attribute macros that make tpt-homestack blueprints "Dead Simple".
//!
//! | Macro | Attaches to | Effect |
//! |---|---|---|
//! | `#[blueprint(id, name)]` | struct | Implements `Blueprint` + registers metadata |
//! | `#[bp_migration]` | `const X: &str` | Registers a namespaced SQL migration |
//! | `#[bp_route(method, path, role?)]` | async fn | Registers an API route under `/api/` |
//! | `#[bp_ui_route(path, name?)]` | fn | Registers a UI page + sidebar entry |
//! | `#[bp_job(every = "5m")]` | async fn | Runs the function periodically in the background |
//! | `#[bp_webhook("feed")]` | async fn | Receives each accepted delivery to a webhook |
//!
//! All registration goes through `inventory`, so the server discovers
//! everything at startup with zero manual wiring. Compile-time checks live
//! here: blueprint ids and route paths are validated, and API routes must
//! sit under `/api/`.

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Ident, ItemConst, ItemFn, ItemStruct, LitStr, Token, parse_macro_input};

/// Key/value attribute args: `key = "value", ...`.
struct KvArgs {
    pairs: Vec<(Ident, LitStr)>,
}

impl Parse for KvArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut pairs = Vec::new();
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let value: LitStr = input.parse()?;
            pairs.push((key, value));
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(Self { pairs })
    }
}

impl KvArgs {
    fn get(&self, key: &str) -> Option<&LitStr> {
        self.pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

/// `#[bp_ui_route]` args: a positional `"/path"` plus optional `name = "..."`.
struct UiArgs {
    path: LitStr,
    name: Option<LitStr>,
}

impl Parse for UiArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path: LitStr = input.parse()?;
        let mut name = None;
        while !input.is_empty() {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            let key: Ident = input.parse()?;
            if key != "name" {
                return Err(syn::Error::new(key.span(), "expected `name = \"...\"`"));
            }
            input.parse::<Token![=]>()?;
            name = Some(input.parse()?);
        }
        Ok(Self { path, name })
    }
}

/// `#[bp_route]` args: `get, "/api/x", role = "admin"? public?`.
struct RouteArgs {
    method: Ident,
    path: LitStr,
    role: Option<LitStr>,
    public: bool,
}

impl Parse for RouteArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let method: Ident = input.parse()?;
        input.parse::<Token![,]>()?;
        let path: LitStr = input.parse()?;
        let mut role = None;
        let mut public = false;
        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            // `public` is not a Rust keyword, so a plain Ident parse covers
            // both `role` and `public`.
            let key: Ident = input.parse()?;
            if key == "public" {
                public = true;
                continue;
            }
            if key == "role" {
                input.parse::<Token![=]>()?;
                role = Some(input.parse()?);
                continue;
            }
            return Err(syn::Error::new(
                key.span(),
                "expected `role = \"...\"` or `public`",
            ));
        }
        Ok(Self {
            method,
            path,
            role,
            public,
        })
    }
}

fn validate_id(id: &str) -> syn::Result<()> {
    let ok = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !id.starts_with(|c: char| c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "blueprint id must be non-empty, lowercase, and made of letters, digits or '_' (no leading digit)",
        ))
    }
}

fn validate_api_path(path: &str) -> syn::Result<()> {
    if path.len() < 5 || !path.starts_with("/api/") {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[bp_route] paths must start with \"/api/\" (blueprints share one router)",
        ));
    }
    Ok(())
}

fn validate_ui_path(path: &str) -> syn::Result<()> {
    if !path.starts_with('/') || path.starts_with("/api/") {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[bp_ui_route] paths are pages: they must start with '/' but must not be under \"/api/\"",
        ));
    }
    Ok(())
}

fn http_method_ident(name: &str) -> Option<Ident> {
    const VALID: [&str; 8] = [
        "get", "post", "put", "delete", "patch", "head", "options", "trace",
    ];
    let lower = name.to_ascii_lowercase();
    VALID
        .contains(&lower.as_str())
        .then(|| Ident::new(&lower, proc_macro2::Span::call_site()))
}

/// Register a blueprint struct: `#[blueprint(id = "myapp", name = "My App")]`.
///
/// Implements [`tpt_home_core::Blueprint`] and submits a `BlueprintEntry` so
/// the server can list the blueprint without any manual registration.
#[proc_macro_attribute]
pub fn blueprint(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as KvArgs);
    let item = parse_macro_input!(item as ItemStruct);

    let Some(id) = args.get("id") else {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "missing `id = \"...\"` in #[blueprint]",
        )
        .to_compile_error()
        .into();
    };
    let Some(name) = args.get("name") else {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "missing `name = \"...\"` in #[blueprint]",
        )
        .to_compile_error()
        .into();
    };
    if let Err(e) = validate_id(&id.value()) {
        return e.to_compile_error().into();
    }

    let ident = &item.ident;
    let expanded = quote! {
        #item

        impl tpt_home_core::Blueprint for #ident {
            fn id(&self) -> &'static str { #id }
            fn name(&self) -> &'static str { #name }
        }

        tpt_home_core::inventory::submit! {
            tpt_home_core::BlueprintEntry {
                id: #id,
                name: #name,
            }
        }
    };
    expanded.into()
}

/// Register a SQL migration: `#[bp_migration] const M: &str = "...";`
///
/// The migration key is `"<crate>:<name>"` where `name` defaults to the const
/// name; override with `#[bp_migration(name = "custom")]`.
#[proc_macro_attribute]
pub fn bp_migration(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as KvArgs);
    let item = parse_macro_input!(item as ItemConst);

    let const_ident = &item.ident;
    let name = args
        .get("name")
        .cloned()
        .unwrap_or_else(|| LitStr::new(&const_ident.to_string(), const_ident.span()));

    let expanded = quote! {
        #item

        tpt_home_core::inventory::submit! {
            tpt_home_core::MigrationEntry {
                key: concat!(env!("CARGO_PKG_NAME"), ":", #name),
                sql: #const_ident,
            }
        }
    };
    expanded.into()
}

/// Register an API route: `#[bp_route(get, "/api/myapp/items")]`.
///
/// The handler may be `async fn` and return `Result<impl IntoResponse,
/// ApiError>`; add `role = "admin"` to require the admin role.
#[proc_macro_attribute]
pub fn bp_route(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as RouteArgs);
    let item = parse_macro_input!(item as ItemFn);

    if let Err(e) = validate_api_path(&args.path.value()) {
        return e.to_compile_error().into();
    }
    let Some(method_fn) = http_method_ident(&args.method.to_string()) else {
        return syn::Error::new(
            args.method.span(),
            "unsupported HTTP method (use get/post/put/delete/patch/head/options/trace)",
        )
        .to_compile_error()
        .into();
    };

    let fn_ident = &item.sig.ident;
    let path_lit = &args.path;
    let method_str = method_fn.to_string().to_ascii_uppercase();

    if args.public && args.role.is_some() {
        return syn::Error::new(
            args.method.span(),
            "`public` routes are unauthenticated; `role` makes no sense with them",
        )
        .to_compile_error()
        .into();
    }

    // Compile-time prefix check: blueprint routes must live under /api/.
    let prefix_check = quote! {
        const _: () = {
            const B: &[u8] = #path_lit.as_bytes();
            const _: () = assert!(
                B.len() >= 5 && B[0] == b'/' && B[1] == b'a' && B[2] == b'p'
                    && B[3] == b'i' && B[4] == b'/',
                "bp_route paths must start with \"/api/\"",
            );
        };
    };

    let registration = if args.public {
        quote! {
            tpt_home_core::inventory::submit! {
                tpt_home_core::PublicRouteEntry {
                    blueprint: env!("CARGO_PKG_NAME"),
                    method: #method_str,
                    path: #path_lit,
                    register: |router| {
                        router.route(#path_lit, axum::routing::#method_fn(#fn_ident))
                    },
                }
            }
        }
    } else {
        let role_expr = match &args.role {
            Some(role) => quote! { ::core::option::Option::Some(#role) },
            None => quote! { ::core::option::Option::None },
        };
        let role_wrap = match &args.role {
            Some(role) => quote! {
                .layer(axum::middleware::from_fn(
                    move |req: axum::extract::Request, next: axum::middleware::Next| async move {
                        tpt_home_core::middleware::role_guard(req, next, #role).await
                    },
                ))
            },
            None => quote! {},
        };
        quote! {
            tpt_home_core::inventory::submit! {
                tpt_home_core::RouteEntry {
                    blueprint: env!("CARGO_PKG_NAME"),
                    method: #method_str,
                    path: #path_lit,
                    role: #role_expr,
                    register: |router| {
                        router.route(#path_lit, axum::routing::#method_fn(#fn_ident)#role_wrap)
                    },
                }
            }
        }
    };

    let expanded = quote! {
        #item

        #prefix_check

        #registration
    };
    expanded.into()
}

/// Register a UI page + sidebar entry:
/// `#[bp_ui_route("/myapp")]` or `#[bp_ui_route("/myapp", name = "My App")]`.
///
/// The handler receives `State(db): State<DbPool>` and returns HTML.
#[proc_macro_attribute]
pub fn bp_ui_route(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as UiArgs);
    let item = parse_macro_input!(item as ItemFn);

    if let Err(e) = validate_ui_path(&args.path.value()) {
        return e.to_compile_error().into();
    }

    let fn_ident = &item.sig.ident;
    let path_lit = &args.path;
    let label = args
        .name
        .clone()
        .unwrap_or_else(|| LitStr::new(&fn_ident.to_string(), fn_ident.span()));

    let expanded = quote! {
        #item

        tpt_home_core::inventory::submit! {
            tpt_home_core::UiEntry {
                blueprint: env!("CARGO_PKG_NAME"),
                path: #path_lit,
                label: #label,
                register: |router| {
                    router.route(#path_lit, axum::routing::get(#fn_ident))
                },
            }
        }
    };
    expanded.into()
}

/// Register a periodic job: `#[bp_job(every = "5m")]` on
/// `async fn name(db: DbPool) -> Result<(), ApiError>`.
///
/// `every` takes a number and a unit: `s`, `m`, `h` or `d` (minimum `10s`).
/// The job runs one period after boot, then once per period. Optional
/// `name = "..."` overrides the log name (default: the function name), and
/// `timeout = "..."` caps one run (default `5m`); a run over the cap is aborted.
#[proc_macro_attribute]
pub fn bp_job(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as KvArgs);
    let item = parse_macro_input!(item as ItemFn);

    let Some(every) = args.get("every") else {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "missing `every = \"...\"` in #[bp_job], e.g. every = \"5m\"",
        )
        .to_compile_error()
        .into();
    };
    let every_secs = match parse_every(&every.value()) {
        Ok(secs) => secs,
        Err(msg) => return syn::Error::new(every.span(), msg).to_compile_error().into(),
    };
    let timeout_secs = match args.get("timeout") {
        Some(text) => match parse_every(&text.value()) {
            Ok(secs) => secs,
            Err(msg) => return syn::Error::new(text.span(), msg).to_compile_error().into(),
        },
        None => DEFAULT_JOB_TIMEOUT_SECS,
    };
    if item.sig.asyncness.is_none() {
        return syn::Error::new(item.sig.span(), "#[bp_job] functions must be `async`")
            .to_compile_error()
            .into();
    }

    let fn_ident = &item.sig.ident;
    let job_name = args
        .get("name")
        .map(LitStr::value)
        .unwrap_or_else(|| fn_ident.to_string());

    let expanded = quote! {
        #item

        tpt_home_core::inventory::submit! {
            tpt_home_core::JobEntry {
                blueprint: env!("CARGO_PKG_NAME"),
                name: #job_name,
                every: ::std::time::Duration::from_secs(#every_secs),
                timeout: ::std::time::Duration::from_secs(#timeout_secs),
                run: |db| ::std::boxed::Box::pin(#fn_ident(db)),
            }
        }
    };
    expanded.into()
}

/// Per-run cap for `#[bp_job]` when no `timeout` is given.
const DEFAULT_JOB_TIMEOUT_SECS: u64 = 300;

/// Parse `"30s"`, `"5m"`, `"1h"` or `"1d"` into seconds. Minimum 10 seconds
/// so a typo cannot turn a job into a busy loop.
fn parse_every(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let (digits, unit) = text.split_at(text.len().saturating_sub(1));
    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        _ => {
            return Err(format!(
                "invalid interval '{text}': unit must be s, m, h or d"
            ));
        }
    };
    let count: u64 = digits
        .parse()
        .map_err(|_| format!("invalid interval '{text}': expected a number before the unit"))?;
    let secs = count
        .checked_mul(multiplier)
        .ok_or_else(|| format!("interval '{text}' is too large"))?;
    if secs < 10 {
        return Err(format!("interval '{text}' is too short: minimum is 10s"));
    }
    Ok(secs)
}

/// Receive webhook deliveries: `#[bp_webhook("feed")]` on
/// `async fn name(db: DbPool, delivery: WebhookDelivery) -> Result<(), ApiError>`.
///
/// The webhook must exist: create it on the Webhooks page (or with
/// `POST /api/webhooks`). The handler runs after the sender has its `202`,
/// once for each accepted delivery, in its own task. Its errors are logged,
/// and a run over a minute is abandoned. The body is the UTF-8 text sent.
#[proc_macro_attribute]
pub fn bp_webhook(attr: TokenStream, item: TokenStream) -> TokenStream {
    let name = parse_macro_input!(attr as LitStr);
    let item = parse_macro_input!(item as ItemFn);

    let value = name.value();
    let well_formed = (1..=50).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !well_formed {
        return syn::Error::new(
            name.span(),
            "webhook names are 1-50 characters of lowercase letters, digits, `_` or `-`",
        )
        .to_compile_error()
        .into();
    }
    if item.sig.asyncness.is_none() {
        return syn::Error::new(item.sig.span(), "#[bp_webhook] functions must be `async`")
            .to_compile_error()
            .into();
    }

    let fn_ident = &item.sig.ident;
    let expanded = quote! {
        #item

        tpt_home_core::inventory::submit! {
            tpt_home_core::WebhookEntry {
                blueprint: env!("CARGO_PKG_NAME"),
                webhook: #value,
                run: |db, delivery| ::std::boxed::Box::pin(#fn_ident(db, delivery)),
            }
        }
    };
    expanded.into()
}

#[cfg(test)]
mod job_tests {
    use super::parse_every;

    #[test]
    fn intervals_parse_to_seconds() {
        assert_eq!(parse_every("30s"), Ok(30));
        assert_eq!(parse_every("5m"), Ok(300));
        assert_eq!(parse_every("1h"), Ok(3_600));
        assert_eq!(parse_every("1d"), Ok(86_400));
        assert_eq!(parse_every(" 2h "), Ok(7_200));
    }

    #[test]
    fn bad_intervals_are_rejected() {
        assert!(parse_every("").is_err());
        assert!(parse_every("5").is_err());
        assert!(parse_every("m").is_err());
        assert!(parse_every("5x").is_err());
        assert!(parse_every("9s").is_err());
        assert!(parse_every("0m").is_err());
        assert!(parse_every("99999999999999999999d").is_err());
    }
}

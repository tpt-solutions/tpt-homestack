// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions
//
// A tpt-homestack plugin in AssemblyScript, following docs/PLUGIN_ABI.md (ABI v1).
// It answers two routes under /api/<id>:
//
//   GET  /hello?name=Ada   -> {"greeting":"Hello, Ada"}
//   POST /hello            -> {"received_bytes":N}
//
// Built with `npm run build` into plugin.wasm, then packed with
// `tpt-home plugin build` (see README.md). The test suite runs this exact file.

// Host import: write a log line (at most 100 per call are kept by the host).
@external("env", "tpt_log")
declare function tpt_log(ptr: usize, len: i32): void;

/** Memory the host may write into. A bump allocator is enough: each request
 *  gets a fresh instance, and `tpt_alloc` is safe to call mid-request. */
export function tpt_alloc(len: i32): i32 {
  return heap.alloc(len) as i32;
}

function text(ptr: i32, len: i32): string {
  return String.UTF8.decodeUnsafe(ptr as usize, len as usize);
}

/** The value of `"key":"..."` in a flat JSON object, or "" when absent. The
 *  value is returned as it appears in the JSON text: escapes are not decoded. */
function field(json: string, key: string): string {
  const marker = '"' + key + '":"';
  const start = json.indexOf(marker);
  if (start < 0) return "";
  const from = start + marker.length;
  // The closing quote is the first one not preceded by a backslash.
  let end = from;
  while (end < json.length && !(json.charAt(end) == '"' && json.charAt(end - 1) != "\\")) {
    end++;
  }
  return end < json.length ? json.substring(from, end) : "";
}

/** The value of `name` in a raw query string such as `name=Ada&x=1`. */
function query_value(query: string, name: string): string {
  const pairs = query.split("&");
  for (let i = 0; i < pairs.length; i++) {
    const pair = pairs[i];
    const eq = pair.indexOf("=");
    if (eq > 0 && pair.substring(0, eq) == name) {
      return pair.substring(eq + 1);
    }
  }
  return "";
}

/** Keep a value safe to put inside a JSON string: letters, digits, spaces. */
function plain(value: string): string {
  let out = "";
  for (let i = 0; i < value.length && i < 64; i++) {
    const c = value.charCodeAt(i);
    const ok =
      (c >= 48 && c <= 57) || (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c == 32;
    if (ok) out += value.charAt(i);
  }
  return out;
}

/** A JSON string literal for `value`, with the characters JSON needs escaped. */
function quote(value: string): string {
  let out = '"';
  for (let i = 0; i < value.length; i++) {
    const c = value.charAt(i);
    if (c == '"' || c == "\\") out += "\\" + c;
    else out += c;
  }
  return out + '"';
}

/** Write a response and return it packed as `(ptr << 32) | len`. */
function reply(status: i32, body: string): i64 {
  const json = String.UTF8.encode(
    '{"status":' + status.toString() + ',"content_type":"application/json","body":' + quote(body) + "}",
  );
  const ptr = heap.alloc(json.byteLength);
  memory.copy(ptr, changetype<usize>(json), json.byteLength);
  return (i64(ptr) << 32) | i64(json.byteLength);
}

function log(message: string): void {
  const bytes = String.UTF8.encode(message);
  tpt_log(changetype<usize>(bytes), bytes.byteLength);
}

/** Handle one request. The host passes `{"method", "path", "query", "body"}`. */
export function tpt_entry(ptr: i32, len: i32): i64 {
  const request = text(ptr, len);
  const method = field(request, "method");
  const path = field(request, "path");
  if (path != "/hello") {
    return reply(404, '{"error":"no such route"}');
  }
  if (method == "GET") {
    const name = plain(query_value(field(request, "query"), "name"));
    log("greeted " + (name.length > 0 ? name : "stranger"));
    return reply(200, '{"greeting":' + quote("Hello, " + (name.length > 0 ? name : "stranger")) + "}");
  }
  if (method == "POST") {
    // The body arrives as a JSON string value or null; its length is what we report.
    const body = field(request, "body");
    return reply(200, '{"received_bytes":' + body.length.toString() + "}");
  }
  return reply(405, '{"error":"method not allowed"}');
}

/** AssemblyScript's bounds and assertion failures call `abort`. The ABI allows
 *  imports from `env` only, and the host does not provide `abort`, so the build
 *  routes it here: a trap, which the host reports as a failed call. */
export function abortTrap(message: usize, fileName: usize, line: u32, column: u32): void {
  unreachable();
}

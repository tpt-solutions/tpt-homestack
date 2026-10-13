// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

// Typings for `runtime = "js"` plugins. Author-side only: the host runs plain
// JavaScript, so transpile TypeScript first or use this file for editor help.
//
// Failure contract: a denied capability or other ordinary failure does not
// throw. Reads return `null`; writes return `false`. A passed deadline ends
// the call instead.

/** Request the host passes to `handle`. */
export interface Request {
  method: string;
  /** Path below `/api/<id>`, for example `/events`. */
  path: string;
  /** Raw query string without the leading `?`. */
  query: string;
  body: string | null;
  /** The signed-in caller. Absent for anonymous calls (none today: plugin routes are protected). */
  user?: { id: number; username: string; role: "admin" | "user" };
}

/** Response returned from `handle`. */
export interface Response {
  status: number;
  content_type?: string;
  body?: string;
}

/** Row objects keyed by column name. */
export type Row = Record<string, string | number | null>;

/** Reply from an outbound request: the status code and the body text. */
export interface HttpReply {
  status: number;
  body: string;
}

/** The clock, in Unix seconds and in the server's local zone. */
export interface Now {
  unix: number;
  /** `YYYY-MM-DD` in the server's local zone. */
  local_date: string;
  local_hour: number;
  local_minute: number;
  utc_offset_minutes: number;
}

export interface Tpt {
  /** Always allowed. Bounded to 100 lines per call. */
  log(...parts: unknown[]): void;

  /** Always allowed. The current time. */
  now(): Now;

  db: {
    /** One scoped SELECT on `<id>_*` tables. `null` when denied or failed. */
    query(sql: string): Row[] | null;
    /** One scoped statement. `false` when denied or failed. */
    execute(sql: string): boolean;
  };

  kv: {
    get(key: string): string | null;
    set(key: string, value: string): boolean;
    del(key: string): boolean;
  };

  /** Post `{"text": message}` to an admin-configured channel. `false` when the channel is not declared, not configured, or did not answer 2xx. Max 2000 characters. */
  notify(channel: string, message: string): boolean;

  http: {
    /** GET a declared `http://` or `https://` URL (https needs a build with the `https` feature). `null` when denied or failed. Redirects are not followed. */
    get(url: string): HttpReply | null;
    /** Plain HTTP POST of a text body, same rules as `get`. */
    post(url: string, body: string): HttpReply | null;
  };

  storage: {
    /** UTF-8 file in the plugin's data directory, or `null` if absent. */
    read(name: string): string | null;
    write(name: string, data: string): boolean;
  };
}

declare global {
  const tpt: Tpt;
  function handle(req: Request): Response;
}

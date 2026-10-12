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
}

/** Response returned from `handle`. */
export interface Response {
  status: number;
  content_type?: string;
  body?: string;
}

/** Row objects keyed by column name. */
export type Row = Record<string, string | number | null>;

export interface Tpt {
  /** Always allowed. Bounded to 100 lines per call. */
  log(...parts: unknown[]): void;

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

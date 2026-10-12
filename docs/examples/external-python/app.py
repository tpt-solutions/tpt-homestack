"""Ticker: an external tpt-homestack plugin in about 40 lines of stdlib Python.

Run with TPT_HOST (the server URL), TPT_TOKEN (a token scoped to plugin:ticker)
and optionally TICKER_PORT (default 8790, matching plugin.toml's entry).
"""
import json
import os
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HOST = os.environ.get("TPT_HOST", "http://127.0.0.1:8080")
TOKEN = os.environ["TPT_TOKEN"]
PORT = int(os.environ.get("TICKER_PORT", "8790"))


def host_call(op, payload):
    """Run one host operation; returns its `value` or raises on denial."""
    req = urllib.request.Request(
        f"{HOST}/api/plugin-host/ticker/{op}",
        data=json.dumps(payload).encode(),
        headers={"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req) as res:
        return json.load(res)["value"]


class Ticker(BaseHTTPRequestHandler):
    def send_json(self, status, payload):
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):  # GET /items
        if self.path.split("?")[0] != "/items":
            return self.send_json(404, {"error": "not found"})
        rows = host_call("db/query", {"sql": "SELECT label FROM ticker_items ORDER BY id"})
        self.send_json(200, rows)

    def do_POST(self):  # POST /items with {"label": "..."}
        if self.path.split("?")[0] != "/items":
            return self.send_json(404, {"error": "not found"})
        data = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        label = str(data["label"]).replace("'", "''")  # quote for SQL
        host_call("db/execute", {"sql": f"INSERT INTO ticker_items (label) VALUES ('{label}')"})
        self.send_json(201, {"ok": True})


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", PORT), Ticker).serve_forever()

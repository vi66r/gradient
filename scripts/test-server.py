#!/usr/bin/env python3
"""Disposable loopback-only pages for real engine / UI verification."""
import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "fixtures"

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        if self.path.startswith("/api/"):
            return self.api()
        name = "gpu.html" if self.path.startswith("/gpu") else "browser.html"
        content = (ROOT / name).read_bytes()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)

    def do_POST(self):
        self.api()

    def api(self):
        body = self.rfile.read(min(int(self.headers.get("Content-Length", 0)), 1024 * 1024))
        content = json.dumps({"method": self.command, "path": self.path,
                              "test_header": self.headers.get("X-Browser-Test"),
                              "body": body.decode("utf-8", errors="replace")}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"Browser fixture: http://127.0.0.1:{server.server_port}", flush=True)
    server.serve_forever()

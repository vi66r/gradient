#!/usr/bin/env python3
"""Disposable loopback-only pages for real engine / UI verification."""
import argparse
import json
import re
import tempfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "fixtures"

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        if self.path.split("?")[0] == "/performance.js":
            content = (ROOT / "performance.js").read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "text/javascript")
            self.end_headers()
            self.wfile.write(content)
            return
        if self.path.startswith("/api/"):
            return self.api()
        name = "gpu.html" if self.path.startswith("/gpu") else "browser.html"
        content = (ROOT / name).read_bytes()
        if self.server.reports is not None:
            content += b'<script src="/performance.js"></script>'
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)

    def do_POST(self):
        if self.path == "/performance-report" and self.server.reports is not None:
            try:
                length = int(self.headers.get("Content-Length", 0))
                if not 0 < length <= 65536:
                    raise ValueError("Invalid report length")
                report = json.loads(self.rfile.read(length))
                run = report["run"]
                if not re.fullmatch(r"[a-zA-Z0-9_-]{1,100}", run):
                    raise ValueError("Invalid run")
                with tempfile.NamedTemporaryFile(mode="w", dir=self.server.reports, delete=False) as output:
                    json.dump(report, output, indent=2)
                Path(output.name).replace(self.server.reports / (run + ".json"))
            except (ValueError, KeyError, TypeError):
                self.send_error(400)
                return
            self.send_response(204)
            self.end_headers()
            return
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
    parser.add_argument("--reports", type=Path, help="Opt-in local performance report directory")
    args = parser.parse_args()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.reports = args.reports
    if args.reports is not None:
        args.reports.mkdir(parents=True, exist_ok=True)
    print(f"Browser fixture: http://127.0.0.1:{server.server_port}", flush=True)
    server.serve_forever()

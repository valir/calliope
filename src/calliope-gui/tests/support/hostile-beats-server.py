#!/usr/bin/env python3
"""QA stand-in for a broken calliope-stems beats server (tests only, loopback only).

  hostile-beats-server.py <port> <payload-file>

Answers /v1/health with tasks ["stems","beats"], accepts POST /v1/beats, reports the job done
immediately and serves the raw bytes of <payload-file> as GET /v1/jobs/<id>/beats.
Binds 127.0.0.1 only. Never reaches any other host.
"""
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT, PAYLOAD = int(sys.argv[1]), sys.argv[2]
JOB = "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b"


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *a):
        pass

    def send(self, code, body, ctype="application/json"):
        if not isinstance(body, bytes):
            body = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.startswith("/v1/health"):
            self.send(200, {"service": "calliope-stems", "api": 1, "version": "qa", "models": ["htdemucs_6s"],
                            "default_model": "htdemucs_6s", "busy": False, "max_duration_s": 900,
                            "max_upload_bytes": 314572800, "tasks": ["stems", "beats"]})
        elif self.path.endswith("/beats"):
            with open(PAYLOAD, "rb") as f:
                self.send(200, f.read())
        elif self.path.startswith("/v1/jobs/"):
            self.send(200, {"job": JOB, "state": "done", "progress": 1.0, "stems": None, "error": None, "kind": "beats"})
        else:
            self.send(404, {"error": "unknown"})

    def do_POST(self):
        n = int(self.headers.get("Content-Length", "0"))
        while n > 0:
            n -= len(self.rfile.read(min(n, 65536)))
        self.send(202, {"job": JOB, "state": "queued"})

    def do_DELETE(self):
        self.send_response(204)
        self.send_header("Connection", "close")
        self.end_headers()


HTTPServer(("127.0.0.1", PORT), H).serve_forever()

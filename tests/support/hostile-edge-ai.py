#!/usr/bin/env python3
"""QA stand-in for a malicious / broken calliope-stems server (tests only, loopback only).

  hostile-edge-ai.py <port> <mode>

modes: traversal (stem list ["../../evil"]), duplicate (["vocals","vocals"]),
       notflac (valid names, bodies are HTML), good (six real stems from the fixtures)
Binds 127.0.0.1 only. Never reaches any other host.
"""
import json, os, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

PORT, MODE = int(sys.argv[1]), sys.argv[2]
HERE = os.path.dirname(os.path.abspath(__file__))
STEMS = os.path.join(HERE, "..", "fixtures", "import", "stems")
JOB = "0190b1c2-3d4e-4f50-8a6b-7c8d9e0f1a2b"
NAMES = {"traversal": ["../../evil"], "duplicate": ["vocals", "vocals"],
         "notflac": ["vocals", "drums"], "good": ["vocals", "drums", "bass", "guitar", "piano", "other"]}[MODE]


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *a):
        sys.stderr.write("hostile-edge-ai: " + fmt % a + "\n")

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
                            "max_upload_bytes": 314572800})
        elif "/stems/" in self.path:
            name = self.path.rsplit("/", 1)[1]
            if MODE == "notflac":
                self.send(200, b"<html>captive portal</html>", "audio/flac")
            else:
                with open(os.path.join(STEMS, name + ".flac"), "rb") as f:
                    self.send(200, f.read(), "audio/flac")
        elif self.path.startswith("/v1/jobs/"):
            self.send(200, {"job": JOB, "state": "done", "progress": 1.0, "stems": NAMES, "error": None})
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

"""A stand-in AI for CI: an OpenAI-compatible chat server on localhost that answers every
request with an empty JSON object. FastMind treats that as "nothing to change" (a note keeps
its captured title, Dream proposes nothing that needs judging), so the walk-through exercises
the real model path, over HTTP, on every OS, without a real model.
"""

import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        self.rfile.read(length)
        body = json.dumps({"choices": [{"message": {"role": "assistant", "content": "{}"}}]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


port = int(sys.argv[1]) if len(sys.argv) > 1 else 8099
HTTPServer(("127.0.0.1", port), Handler).serve_forever()

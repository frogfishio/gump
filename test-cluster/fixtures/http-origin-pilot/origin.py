import hmac
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


PORT = 18081
DOMAINS = ["origin.gump.test", "alternate.gump.test", "gump.frogfish.io"]
MEDIA_TYPE = "application/vnd.gump.hiccup+json; version=1"


def read_hiccup_token():
    raw_fd = os.environ.get("GUMP_HICCUP_TOKEN_FD")
    if raw_fd is None:
        raise SystemExit("GUMP_HICCUP_TOKEN_FD is required")
    fd = int(raw_fd)
    try:
        os.lseek(fd, 0, os.SEEK_SET)
        token = os.read(fd, 33)
    finally:
        os.close(fd)
    if len(token) != 32:
        raise SystemExit("Hiccup token must be exactly 32 bytes")
    return "Hiccup " + token.hex()


EXPECTED_AUTHORIZATION = read_hiccup_token()
DECLARATION = json.dumps(
    {
        "hiccup": 1,
        "capabilities": {
            "http.origin/1": {"port": PORT, "domains": DOMAINS},
        },
    },
    separators=(",", ":"),
).encode()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def send_body(self, status, body, content_type):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            if self.headers.get("Hiccup-Offer") == "1":
                self.send_body(200, DECLARATION, MEDIA_TYPE)
            else:
                self.send_body(200, b'{"status":"live"}', "application/json")
            return
        body = json.dumps(
            {
                "status": "origin-ok",
                "host": self.headers.get("Host", ""),
                "forwardedHost": self.headers.get("X-Forwarded-Host", ""),
                "localAddress": self.connection.getsockname()[0],
            },
            separators=(",", ":"),
        ).encode()
        self.send_body(200, body, "application/json")

    def do_POST(self):
        if self.path != "/health":
            self.send_body(404, b"not found\n", "text/plain")
            return
        supplied = self.headers.get("Authorization", "")
        if not hmac.compare_digest(supplied, EXPECTED_AUTHORIZATION):
            self.send_body(401, b"unauthorized\n", "text/plain")
            return
        try:
            size = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            size = -1
        if size < 0 or size > 256 * 1024:
            self.send_body(413, b"too large\n", "text/plain")
            return
        try:
            delivery = json.loads(self.rfile.read(size))
        except (UnicodeDecodeError, json.JSONDecodeError):
            self.send_body(400, b"bad delivery\n", "text/plain")
            return
        if delivery.get("hiccup") != 1 or not isinstance(delivery.get("messages"), list):
            self.send_body(400, b"bad delivery\n", "text/plain")
            return
        self.send_body(200, DECLARATION, MEDIA_TYPE)

    def log_message(self, _format, *_args):
        return


ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()

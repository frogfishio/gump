#!/usr/bin/env python3

import argparse
import ipaddress
import json
import re
import urllib.error
import urllib.parse
import urllib.request
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


HOST_RE = re.compile(r"^(?=.{1,253}$)(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$")


def normalize_domain(raw_domain: str) -> str | None:
    domain = raw_domain.strip().lower().rstrip(".")
    if not domain:
        return None

    bracketless = domain[1:-1] if domain.startswith("[") and domain.endswith("]") else domain
    try:
        ipaddress.ip_address(bracketless)
        return None
    except ValueError:
        pass

    if not HOST_RE.fullmatch(domain):
        return None

    return domain


def consul_allows_domain(consul_http: str, domain: str, timeout_seconds: float) -> bool:
    encoded_domain = urllib.parse.quote(domain, safe="")
    url = f"{consul_http.rstrip('/')}/v1/query/{encoded_domain}/execute"
    request = urllib.request.Request(url, headers={"Accept": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            if response.status != HTTPStatus.OK:
                return False
            payload = json.load(response)
    except (TimeoutError, urllib.error.URLError, urllib.error.HTTPError, json.JSONDecodeError):
        return False

    nodes = payload.get("Nodes")
    if not isinstance(nodes, list) or not nodes:
        return False

    for node in nodes:
        if not isinstance(node, dict):
            continue
        service = node.get("Service")
        if not isinstance(service, dict):
            continue
        service_meta = service.get("Meta")
        if isinstance(service_meta, dict) and service_meta.get("domain") == domain:
            return True

    return False


class AllowHandler(BaseHTTPRequestHandler):
    consul_http = "http://127.0.0.1:8500"
    timeout_seconds = 2.0

    def do_GET(self) -> None:
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path != "/allow":
            self.send_error(HTTPStatus.NOT_FOUND)
            return

        params = urllib.parse.parse_qs(parsed.query)
        raw_domain = params.get("domain", [""])[0]
        domain = normalize_domain(raw_domain)
        if domain is None:
            self.send_response(HTTPStatus.FORBIDDEN)
            self.end_headers()
            self.wfile.write(b"denied\n")
            return

        if consul_allows_domain(self.consul_http, domain, self.timeout_seconds):
            self.send_response(HTTPStatus.OK)
            self.end_headers()
            self.wfile.write(b"allowed\n")
            return

        self.send_response(HTTPStatus.FORBIDDEN)
        self.end_headers()
        self.wfile.write(b"denied\n")

    def log_message(self, format: str, *args) -> None:
        return


def main() -> None:
    parser = argparse.ArgumentParser(description="Allow Caddy on-demand TLS only for Consul-backed domains.")
    parser.add_argument("--listen-host", default="127.0.0.1")
    parser.add_argument("--listen-port", type=int, default=9123)
    parser.add_argument("--consul-http", default="http://127.0.0.1:8500")
    parser.add_argument("--timeout-seconds", type=float, default=2.0)
    args = parser.parse_args()

    AllowHandler.consul_http = args.consul_http
    AllowHandler.timeout_seconds = args.timeout_seconds

    server = ThreadingHTTPServer((args.listen_host, args.listen_port), AllowHandler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
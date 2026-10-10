#!/usr/bin/env python3
"""Serve a directory with the headers Wasm threads need (local testing of web/dist).

    scripts/web-serve.py web/dist 8765

SharedArrayBuffer only exists on cross-origin isolated pages, so every response carries
COOP/COEP (the same headers web/_headers sets on Cloudflare).
"""

import functools
import http.server
import sys


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
    }

    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cross-Origin-Resource-Policy", "same-origin")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else "web/dist"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 8765
    handler = functools.partial(Handler, directory=root)
    with http.server.ThreadingHTTPServer(("127.0.0.1", port), handler) as httpd:
        print(f"serving {root} on http://127.0.0.1:{port}", flush=True)
        httpd.serve_forever()


if __name__ == "__main__":
    main()

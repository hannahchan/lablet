"""A listener for the reference's OTLP/HTTP export that keeps nothing.

Claude Code writes its request-body log only while an OTel logs exporter is
on, and the exporter wants somewhere to send to. This answers every POST
with 200 and an empty body, which is a valid empty export response, and
discards what arrived. It prints the path and the size of each body, never
a header and never the body, so nothing of the capture reaches the terminal.

Standard library only. Run it with `python3 -I otlp_sink.py [--port 4318]`.
"""

from __future__ import annotations

import argparse
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer


class Sink(BaseHTTPRequestHandler):
    # The server's own log line would print the request line, which is
    # harmless, but one printer is easier to audit than two.
    def log_message(self, format, *args):  # noqa: A002 - the base class's name
        return

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        remaining = length
        while remaining > 0:
            chunk = self.rfile.read(min(remaining, 65536))
            if not chunk:
                break
            remaining -= len(chunk)
        self.send_response(200)
        self.send_header("Content-Type", "application/x-protobuf")
        self.send_header("Content-Length", "0")
        self.end_headers()
        print("sink: POST %s, %d bytes, dropped" % (self.path, length), flush=True)

    def do_GET(self):
        self.send_response(404)
        self.send_header("Content-Length", "0")
        self.end_headers()


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port", type=int, default=4318)
    args = parser.parse_args(argv)
    server = HTTPServer(("127.0.0.1", args.port), Sink)
    print("sink: listening on http://127.0.0.1:%d, keeping nothing" % args.port, flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

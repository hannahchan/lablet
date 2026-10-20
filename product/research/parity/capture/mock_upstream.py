"""A stand-in for the Messages API that plays one status sequence.

For run R5: the reference is pointed at this with `ANTHROPIC_BASE_URL` and a
dummy key, and the sequence says what each request in turn gets. A 200 is a
short message streamed as server-sent events, since the reference streams and
would otherwise never return. Once the sequence is used up, its last step
repeats, so `429-retry-after-61` keeps asking for 61 seconds and `529x12`
keeps failing.

The log, one line per request on standard output: the time, the status, the
path and the two retry headers this sent, `retry-after` and `x-should-retry`.
Never a request header, so the key, dummy or not, is never printed, and never
a request body.

The sequence grammar, as the capture's README spells the six:

    <step>[-then-<step>]...
    <step> = <status>[x<count>][-retry-after-<seconds>][-x-should-retry-<true|false>][-prompt-too-long]

so `529x12`, `500x3-then-200`, `429-retry-after-7-then-200`, `429-retry-after-61`,
`500-x-should-retry-false` and `400-prompt-too-long-then-200`.

Standard library only. Run it with `python3 -I mock_upstream.py --sequence 529x12 [--port 8765]`.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

STEP = re.compile(
    r"^(?P<status>\d{3})"
    r"(?:x(?P<count>\d+))?"
    r"(?P<flags>(?:-retry-after-\d+|-x-should-retry-(?:true|false)|-prompt-too-long)*)$"
)

ERROR_TYPES = {
    400: ("invalid_request_error", "Invalid request"),
    401: ("authentication_error", "invalid x-api-key"),
    403: ("permission_error", "Your API key does not have permission"),
    404: ("not_found_error", "Not found"),
    413: ("request_too_large", "Request exceeds the maximum allowed size"),
    429: ("rate_limit_error", "This request would exceed your organization's rate limit"),
    500: ("api_error", "An unexpected error has occurred internal to Anthropic's systems"),
    529: ("overloaded_error", "Overloaded"),
}

PROMPT_TOO_LONG = "prompt is too long: 250000 tokens > 200000 maximum"


def parse_sequence(text):
    """The steps of `text`, each a dict of status, retry_after, x_should_retry
    and prompt_too_long, with an `x<count>` step repeated count times."""
    steps = []
    for part in text.split("-then-"):
        match = STEP.match(part)
        if not match:
            raise ValueError("can't read the step %r of the sequence %r" % (part, text))
        step = {
            "status": int(match.group("status")),
            "retry_after": None,
            "x_should_retry": None,
            "prompt_too_long": False,
        }
        flags = match.group("flags")
        retry_after = re.search(r"-retry-after-(\d+)", flags)
        if retry_after:
            step["retry_after"] = int(retry_after.group(1))
        should_retry = re.search(r"-x-should-retry-(true|false)", flags)
        if should_retry:
            step["x_should_retry"] = should_retry.group(1)
        step["prompt_too_long"] = "-prompt-too-long" in flags
        steps.extend([step] * int(match.group("count") or 1))
    return steps


def sse(model):
    """A streamed message of one short text block that ended the turn."""
    events = [
        ("message_start", {
            "type": "message_start",
            "message": {
                "id": "msg_mock_0000000000000000000001",
                "type": "message",
                "role": "assistant",
                "model": model,
                "content": [],
                "stop_reason": None,
                "stop_sequence": None,
                "usage": {"input_tokens": 25, "output_tokens": 1},
            },
        }),
        ("content_block_start", {
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "text", "text": ""},
        }),
        ("content_block_delta", {
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": "ready"},
        }),
        ("content_block_stop", {"type": "content_block_stop", "index": 0}),
        ("message_delta", {
            "type": "message_delta",
            "delta": {"stop_reason": "end_turn", "stop_sequence": None},
            "usage": {"output_tokens": 3},
        }),
        ("message_stop", {"type": "message_stop"}),
    ]
    out = []
    for name, data in events:
        out.append("event: %s\ndata: %s\n\n" % (name, json.dumps(data)))
    return "".join(out).encode("utf-8")


class Upstream(BaseHTTPRequestHandler):
    steps = []
    served = 0
    lock = threading.Lock()

    def log_message(self, format, *args):  # noqa: A002 - the base class's name
        return

    def next_step(self):
        with Upstream.lock:
            index = min(Upstream.served, len(Upstream.steps) - 1)
            Upstream.served += 1
        return Upstream.steps[index]

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        model = "unknown"
        try:
            model = json.loads(body).get("model", model)
        except (ValueError, AttributeError):
            pass
        step = self.next_step()
        status = step["status"]
        headers = []
        if step["retry_after"] is not None:
            headers.append(("retry-after", str(step["retry_after"])))
        if step["x_should_retry"] is not None:
            headers.append(("x-should-retry", step["x_should_retry"]))
        if status == 200:
            payload = sse(model)
            content_type = "text/event-stream"
        else:
            kind, message = ERROR_TYPES.get(status, ("api_error", "Error"))
            if step["prompt_too_long"]:
                message = PROMPT_TOO_LONG
            payload = json.dumps({
                "type": "error",
                "error": {"type": kind, "message": message},
                "request_id": "req_mock_%08d" % Upstream.served,
            }).encode("utf-8")
            content_type = "application/json"
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("request-id", "req_mock_%08d" % Upstream.served)
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(payload)
        sent = ", ".join("%s: %s" % header for header in headers) or "no retry headers"
        print(
            "%s %d %s %s"
            % (time.strftime("%Y-%m-%dT%H:%M:%S"), status, self.path, sent),
            flush=True,
        )

    def do_GET(self):
        self.send_response(404)
        self.send_header("Content-Length", "0")
        self.end_headers()
        print("%s 404 %s" % (time.strftime("%Y-%m-%dT%H:%M:%S"), self.path), flush=True)


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--sequence", required=True, help="such as 500x3-then-200")
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args(argv)
    Upstream.steps = parse_sequence(args.sequence)
    server = HTTPServer(("127.0.0.1", args.port), Upstream)
    print(
        "mock upstream: listening on http://127.0.0.1:%d, playing %s (%d steps, the last repeats)"
        % (args.port, args.sequence, len(Upstream.steps)),
        flush=True,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

#!/usr/bin/env python3

"""
Minimal SSE proxy for the OpenAI Responses API at /v1/responses.

Two modes:
  1) Default passthrough: forwards upstream SSE events as-is.
  2) --final-only: suppresses streaming deltas and emits only one final
     `response.output_item.done` (aggregated assistant text) followed by
     `response.completed`. This is useful to reproduce the Codex TUI issue
     where no messages render when only a final item is received.

Additionally logs the reconstructed assistant output on the server for
visibility.

Point Codex to this server by defining a provider with base_url pointing to
http://127.0.0.1:PORT/v1 and wire_api = "responses".

Example ~/.codex/config.toml snippet:

    model = "o4-mini"
    model_provider = "local-proxy"

    [model_providers.local-proxy]
    name = "Local Responses Proxy"
    base_url = "http://127.0.0.1:18080/v1"
    env_key = "OPENAI_API_KEY"   # required by upstream; read by this server
    wire_api = "responses"

Run:
  pip install requests
  python3 test-server.py --port 18080 [--final-only]

Server logs:
  - Each SSE event type as it arrives
  - Aggregated assistant text on completion
"""

from __future__ import annotations

import argparse
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import List, Optional, Tuple
import time
import os

try:
    import requests  # type: ignore
except Exception:
    requests = None  # resolved at runtime; we print a helpful error


class ResponsesHandler(BaseHTTPRequestHandler):
    server_version = "ResponsesProxy/0.2"
    final_only: bool = False
    upstream_base_url: str = "https://api.openai.com/v1"
    bearer_env: str = "OPENAI_API_KEY"

    def log_message(self, fmt: str, *args) -> None:  # quieter logs
        sys.stderr.write("%s - - [%s] " % (self.address_string(), self.log_date_time_string()))
        sys.stderr.write((fmt % args) + "\n")

    def _read_json(self) -> dict:
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length) if length > 0 else b"{}"
        try:
            return json.loads(body.decode("utf-8"))
        except Exception:
            return {}

    def do_POST(self) -> None:  # noqa: N802 – required by BaseHTTPRequestHandler
        # Accept both /responses and /v1/responses for convenience
        if self.path not in ("/responses", "/v1/responses"):
            self.send_error(404, "Not Found")
            return

        if requests is None:
            self.send_error(500, "requests library is required. Run: pip install requests")
            return

        payload = self._read_json()
        # Ensure streaming enabled upstream
        payload["stream"] = True
        self.log_message("Received POST %s; final-only=%s", self.path, ResponsesHandler.final_only)

        api_key = os.environ.get(self.bearer_env, "").strip()
        if not api_key:
            self.send_error(401, f"Missing {self.bearer_env} in environment for upstream auth")
            return

        headers = {
            "Authorization": f"Bearer {api_key}",
            "OpenAI-Beta": "responses=experimental",
            "Accept": "text/event-stream",
            "Content-Type": "application/json",
        }

        upstream_url = f"{self.upstream_base_url}/responses"
        try:
            upstream = requests.post(
                upstream_url,
                headers=headers,
                json=payload,
                stream=True,
                timeout=(10, None),  # connect timeout, stream without read timeout
            )
        except Exception as e:
            self.send_error(502, f"Upstream error: {e}")
            return

        if upstream.status_code < 200 or upstream.status_code >= 300:
            # Try to surface upstream error body
            try:
                body = upstream.text
            except Exception:
                body = ""
            self.send_response(upstream.status_code)
            self.send_header("Content-Type", upstream.headers.get("Content-Type", "text/plain"))
            self.end_headers()
            self.wfile.write(body.encode("utf-8", errors="ignore"))
            return

        # Prepare SSE response headers to our client
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()

        # Simple SSE framing: accumulate lines until a blank line terminates an event
        buf_lines: List[str] = []
        aggregated_text: List[str] = []
        aggregated_reasoning_summary: List[str] = []
        saw_reasoning_summary_done: bool = False
        last_completed: Optional[dict] = None

        def flush_downstream(event_type: str, data: Optional[dict]) -> None:
            self.wfile.write(f"event: {event_type}\n".encode("utf-8"))
            if data is not None:
                payload = json.dumps(data, separators=(",", ":"))
                self.wfile.write(f"data: {payload}\n\n".encode("utf-8"))
            else:
                self.wfile.write(b"\n")
            self.wfile.flush()
            self.log_message("SSE -> %s", event_type)

        def handle_event(block: str) -> None:
            nonlocal last_completed
            # Parse a single SSE block (possibly multiple data: lines)
            etype: Optional[str] = None
            data_lines: List[str] = []
            for line in block.splitlines():
                if line.startswith("event:"):
                    etype = line[len("event:"):].strip()
                elif line.startswith("data:"):
                    data_lines.append(line[len("data:"):].lstrip())
            data_obj: Optional[dict] = None
            if data_lines:
                try:
                    data_obj = json.loads("\n".join(data_lines))
                except Exception:
                    data_obj = None

            if not etype:
                return

            # Logging and aggregation
            self.log_message("SSE <- %s", etype)
            if etype == "response.output_text.delta" and data_obj and "delta" in data_obj:
                delta = data_obj.get("delta", "")
                aggregated_text.append(delta)
            elif etype == "response.output_item.done" and data_obj:
                item = data_obj.get("item", {})
                if item.get("type") == "message" and item.get("role") == "assistant":
                    for c in item.get("content", []) or []:
                        if c.get("type") == "output_text":
                            aggregated_text.append(c.get("text", ""))
            elif etype == "response.reasoning_summary_text.delta" and data_obj and "delta" in data_obj:
                aggregated_reasoning_summary.append(data_obj.get("delta", ""))
            elif etype == "response.reasoning_summary_text.done":
                if aggregated_reasoning_summary:
                    self.log_message(
                        "Reasoning summary finalized: %s",
                        "".join(aggregated_reasoning_summary),
                    )
                else:
                    self.log_message("Reasoning summary finalized (no deltas captured)")
                saw_reasoning_summary_done = True
            elif etype == "response.completed" and data_obj:
                last_completed = data_obj  # capture id/usage

            # Forwarding
            if not ResponsesHandler.final_only:
                # passthrough mode: forward all events
                flush_downstream(etype, data_obj)
            else:
                # final-only mode: only forward created; suppress deltas and items until completed
                if etype == "response.created":
                    flush_downstream(etype, data_obj)
                elif etype == "response.completed":
                    # Emit one synthesized final message (if any), then completed
                    full_text = "".join(aggregated_text)
                    if full_text:
                        synthetic_item = {
                            "type": "response.output_item.done",
                            "item": {
                                "type": "message",
                                "role": "assistant",
                                "content": [{"type": "output_text", "text": full_text}],
                            },
                        }
                        flush_downstream("response.output_item.done", synthetic_item)
                    flush_downstream("response.completed", data_obj)

        try:
            for raw in upstream.iter_lines(decode_unicode=True):
                # requests splits on \n – preserve empty lines as block terminators
                line = raw if isinstance(raw, str) else raw.decode("utf-8", errors="ignore")
                if line == "":
                    if buf_lines:
                        handle_event("\n".join(buf_lines))
                        buf_lines.clear()
                    # else: spurious blank
                else:
                    buf_lines.append(line)
            # Flush remaining
            if buf_lines:
                handle_event("\n".join(buf_lines))
                buf_lines.clear()
        finally:
            # Summarize on server logs
            final_text = "".join(aggregated_text)
            self.log_message("Aggregated assistant output: %s", final_text)
            final_reasoning = "".join(aggregated_reasoning_summary)
            if saw_reasoning_summary_done or final_reasoning:
                self.log_message("Aggregated reasoning summary: %s", final_reasoning)
            # Ensure client connection closes cleanly
            try:
                self.wfile.flush()
            except Exception:
                pass

    def do_GET(self) -> None:  # simple health check
        self.log_message("Received GET request with path: %s", self.path)
        if self.path in ("/health", "/", "/v1/health"):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b"{\"ok\":true}")
            self.log_message("Health check successful.")
        else:
            self.send_error(404, "Not Found")
            self.log_message("Health check failed: Not Found")


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="Minimal Responses SSE proxy for testing")
    ap.add_argument("--host", default="127.0.0.1", help="bind host (default: 127.0.0.1)")
    ap.add_argument("--port", type=int, default=18080, help="bind port (default: 18080)")
    ap.add_argument("--upstream", default="https://api.openai.com/v1", help="upstream base URL (default: https://api.openai.com/v1)")
    ap.add_argument("--bearer-env", default="OPENAI_API_KEY", help="env var for upstream API key (default: OPENAI_API_KEY)")
    ap.add_argument("--final-only", action="store_true", help="suppress deltas and emit only a final message + completed")
    args = ap.parse_args(argv)

    # Configure class-level switches for the handler
    ResponsesHandler.final_only = bool(args.final_only)
    ResponsesHandler.upstream_base_url = str(args.upstream).rstrip("/")
    ResponsesHandler.bearer_env = str(args.bearer_env)

    httpd = HTTPServer((args.host, args.port), ResponsesHandler)
    print(f"Test Responses server listening on http://{args.host}:{args.port}")
    mode = "final-only" if args.final_only else "passthrough"
    print(f"Mode: {mode}; Upstream: {ResponsesHandler.upstream_base_url}/responses; Auth env: {ResponsesHandler.bearer_env}")
    print("Endpoints: POST /v1/responses (SSE), GET /health")
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("Server interrupted by user.")
    finally:
        httpd.server_close()
        print("Server closed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

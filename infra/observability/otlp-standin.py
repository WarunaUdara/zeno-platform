"""A stand-in for the metadata server and the Cloud Trace OTLP endpoint.

Point a Zeno process at this and the whole export path becomes observable without a
GCP project: the token fetch, the `Metadata-Flavor` header the metadata server
requires, the bearer token the exporter attaches, and the span payloads themselves.

Two variables do it:

    GCE_METADATA_HOST=http://127.0.0.1:4599
    OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4599

The metadata host is Google's own convention for relocating the metadata client, so
this is the supported seam rather than a test hook invented for the occasion.

`verify-trace-export.sh` drives this; `GET /__state` is the assertion surface.

Bodies are appended to a log as length-prefixed frames so both the Rust exporter
(protobuf) and the Node exporter (JSON, chunked) can be recorded by one process.
"""

import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

STATE = {
    "token_requests": 0,
    "token_metadata_flavor": None,
    "token_path": None,
    "export_requests": 0,
    "export_path": None,
    "export_authorization": None,
    "export_content_type": None,
    "export_transfer_encoding": None,
    "export_bytes": 0,
}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def _json(self, payload, status=200):
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.endswith("/token"):
            STATE["token_requests"] += 1
            STATE["token_metadata_flavor"] = self.headers.get("Metadata-Flavor")
            STATE["token_path"] = self.path
            self._json(
                {
                    "access_token": "local-standin-token",
                    "expires_in": 3599,
                    "token_type": "Bearer",
                }
            )
            return
        if self.path == "/__state":
            self._json(STATE)
            return
        self._json({"error": "not found"}, status=404)

    def _read_body(self):
        """Read the body, chunked or not.

        The Node exporter sends `Transfer-Encoding: chunked` with no Content-Length.
        Reading only Content-Length records an empty body, which is indistinguishable
        from an exporter that posted nothing -- a green-looking harness that proves
        the opposite of what it claims.
        """
        if self.headers.get("transfer-encoding", "").lower() == "chunked":
            chunks = []
            while True:
                size = int(self.rfile.readline().strip().split(b";")[0] or b"0", 16)
                if size == 0:
                    self.rfile.readline()
                    break
                chunks.append(self.rfile.read(size))
                self.rfile.readline()
            return b"".join(chunks)

        return self.rfile.read(int(self.headers.get("Content-Length", 0)))

    def do_POST(self):
        body = self._read_body()
        STATE["export_requests"] += 1
        STATE["export_path"] = self.path
        STATE["export_authorization"] = self.headers.get("authorization")
        STATE["export_content_type"] = self.headers.get("content-type")
        STATE["export_transfer_encoding"] = self.headers.get("transfer-encoding")
        STATE["export_bytes"] += len(body)

        with open(BODY_LOG, "ab") as handle:
            handle.write(len(body).to_bytes(4, "big"))
            handle.write(body)

        # An empty ExportTraceServiceResponse is a valid acknowledgement.
        self.send_response(200)
        self.send_header("Content-Type", "application/x-protobuf")
        self.send_header("Content-Length", "0")
        self.end_headers()


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: otlp-standin.py <port> <body-log>")
    PORT = int(sys.argv[1])
    BODY_LOG = sys.argv[2]
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()

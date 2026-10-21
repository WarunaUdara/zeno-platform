import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  reservationTraceparent,
  resolveOutboundTraceparent,
  traceparentFor,
} from "./trace-context";

const TRACEPARENT_PATTERN = /^00-(?!0{32})[0-9a-f]{32}-(?!0{16})[0-9a-f]{16}-[0-9a-f]{2}$/;

describe("reservation trace context", () => {
  it("keeps a valid W3C context and clears reserved flags", () => {
    assert.equal(
      reservationTraceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-ff"),
      "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
    );
  });

  it("replaces malformed and all-zero identifiers with a valid root context", () => {
    for (const incoming of [
      "not-a-trace",
      "00-00000000000000000000000000000000-00f067aa0ba902b7-00",
      "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-00",
    ]) {
      const generated = reservationTraceparent(incoming);
      assert.match(generated, /^00-(?!0{32})[0-9a-f]{32}-(?!0{16})[0-9a-f]{16}-00$/);
    }
  });
});

describe("outbound API trace context", () => {
  it("formats an active span as a traceparent the API will accept", () => {
    assert.equal(
      traceparentFor({
        traceId: "4bf92f3577b34da6a3ce929d0e0e4736",
        spanId: "00f067aa0ba902b7",
        traceFlags: 1,
        isRemote: false,
      }),
      "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
    );
  });

  it("narrows wider trace flags to the two digits the header format allows", () => {
    const formatted = traceparentFor({
      traceId: "4bf92f3577b34da6a3ce929d0e0e4736",
      spanId: "00f067aa0ba902b7",
      traceFlags: 0xff,
      isRemote: false,
    });
    assert.equal(formatted, "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-ff");
    assert.match(formatted ?? "", TRACEPARENT_PATTERN);
  });

  it("refuses an unusable span context rather than emitting a broken header", () => {
    assert.equal(traceparentFor(undefined), undefined);
    assert.equal(
      traceparentFor({
        traceId: "00000000000000000000000000000000",
        spanId: "00f067aa0ba902b7",
        traceFlags: 0,
        isRemote: false,
      }),
      undefined,
    );
  });

  it("preserves the caller parent span so the API joins the existing trace", () => {
    const headers = new Headers({
      traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
    });
    assert.equal(
      resolveOutboundTraceparent(headers),
      "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
    );
  });

  it("starts a valid root span when the caller has no trace context", () => {
    // Organizer server components and server actions reach the API without one.
    // They must still produce a traceable span rather than an untraced request.
    const generated = resolveOutboundTraceparent(new Headers());
    assert.match(generated, /^00-(?!0{32})[0-9a-f]{32}-(?!0{16})[0-9a-f]{16}-00$/);
  });

  it("generates a distinct span per request", () => {
    const first = resolveOutboundTraceparent(new Headers());
    const second = resolveOutboundTraceparent(new Headers());
    assert.notEqual(first, second);
  });
});

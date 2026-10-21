import { randomBytes } from "node:crypto";
import { isSpanContextValid, trace, type SpanContext } from "@opentelemetry/api";

const TRACEPARENT_V00 = /^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$/;

function randomIdentifier(byteCount: 8 | 16): string {
  let identifier: string;
  do {
    identifier = randomBytes(byteCount).toString("hex");
  } while (/^0+$/.test(identifier));
  return identifier;
}

export function reservationTraceparent(incoming: string | null): string {
  const match = incoming?.match(TRACEPARENT_V00);
  if (
    match &&
    !/^0+$/.test(match[1]) &&
    !/^0+$/.test(match[2])
  ) {
    const flags = Number.parseInt(match[3], 16) & 1;
    return `00-${match[1]}-${match[2]}-${flags.toString(16).padStart(2, "0")}`;
  }

  return `00-${randomIdentifier(16)}-${randomIdentifier(8)}-00`;
}

/**
 * The W3C `traceparent` for a span context, or nothing if the context is unusable.
 *
 * `traceFlags` is narrowed to two hex digits because that is what the header format
 * specifies; a wider value would produce a header the API's own validator rejects,
 * and the two tiers would silently stop agreeing on the trace.
 */
export function traceparentFor(spanContext: SpanContext | undefined): string | undefined {
  if (!spanContext || !isSpanContextValid(spanContext)) {
    return undefined;
  }

  const flags = spanContext.traceFlags & 0xff;
  return `00-${spanContext.traceId}-${spanContext.spanId}-${flags.toString(16).padStart(2, "0")}`;
}

/**
 * Choose the trace context for an outgoing API call.
 *
 * The active span wins when the tracer is running, because the web tier exports its
 * own spans: the API's span should nest under the web tier's root span so one trace
 * covers the whole request, rather than the API joining a trace nobody records.
 * When no provider is installed — local runs, tests, CI — there is no active span and
 * this falls back to the caller's context, or to a fresh root so no web -> API hop is
 * ever untraced.
 */
export function resolveOutboundTraceparent(headers: Headers): string {
  return (
    traceparentFor(trace.getActiveSpan()?.spanContext()) ??
    headers.get("traceparent") ??
    reservationTraceparent(null)
  );
}

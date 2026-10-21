import { NextResponse } from "next/server";
import { createServerApiClient } from "@/lib/zeno-api";
import { reservationTraceparent } from "@/lib/trace-context";

function problem(status: number, title: string, detail: string, traceparent: string) {
  return NextResponse.json(
    { type: "about:blank", title, status, detail },
    {
      status,
      headers: {
        "Content-Type": "application/problem+json",
        "Cache-Control": "no-store",
        traceparent,
      },
    },
  );
}

export async function POST(request: Request) {
  const traceparent = reservationTraceparent(request.headers.get("traceparent"));
  const maxBodyBytes = 2048;
  const contentLength = Number(request.headers.get("content-length"));
  if (Number.isFinite(contentLength) && contentLength > maxBodyBytes) {
    return problem(413, "Request too large", "The reservation request is too large.", traceparent);
  }

  const idempotencyKey = request.headers.get("Idempotency-Key") ?? "";
  if (!/^[A-Za-z0-9_-]{22,128}$/.test(idempotencyKey)) {
    return problem(422, "Invalid idempotency key", "Please retry this reservation.", traceparent);
  }

  let payload: unknown;
  try {
    const reader = request.body?.getReader();
    if (!reader) {
      return problem(400, "Invalid request", "The reservation details could not be read.", traceparent);
    }

    const chunks: Uint8Array[] = [];
    let totalBytes = 0;
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      totalBytes += value.byteLength;
      if (totalBytes > maxBodyBytes) {
        await reader.cancel();
        return problem(413, "Request too large", "The reservation request is too large.", traceparent);
      }
      chunks.push(value);
    }

    const body = new Uint8Array(totalBytes);
    let offset = 0;
    for (const chunk of chunks) {
      body.set(chunk, offset);
      offset += chunk.byteLength;
    }
    payload = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(body));
  } catch {
    return problem(400, "Invalid request", "The reservation details could not be read.", traceparent);
  }

  if (typeof payload !== "object" || payload === null) {
    return problem(422, "Invalid request", "Choose a ticket and quantity.", traceparent);
  }

  const values = payload as Record<
    string,
    unknown
  >;
  const { event_id: eventId, ticket_type_id: ticketTypeId, quantity } = values;
  if (
    Object.keys(values).length !== 3 ||
    !Object.hasOwn(values, "event_id") ||
    !Object.hasOwn(values, "ticket_type_id") ||
    !Object.hasOwn(values, "quantity") ||
    typeof ticketTypeId !== "string" ||
    !/^zeno_[A-Za-z0-9]{16,32}$/.test(ticketTypeId) ||
    typeof eventId !== "string" ||
    !/^zeno_[A-Za-z0-9]{16,32}$/.test(eventId) ||
    !Number.isInteger(quantity) ||
    (quantity as number) < 1 ||
    (quantity as number) > 100
  ) {
    return problem(422, "Invalid reservation", "Check the ticket and quantity.", traceparent);
  }

  try {
    const client = createServerApiClient();
    const { response, data, error } = await client.POST(
      "/v1/events/{event_id}/ticket-types/{ticket_type_id}/reservations",
      {
        params: {
          path: { event_id: eventId, ticket_type_id: ticketTypeId },
          header: {
            "Idempotency-Key": idempotencyKey,
            traceparent,
          },
        },
        body: { quantity: quantity as number },
        signal: AbortSignal.timeout(8000),
      },
    );
    const headers = new Headers({
      "Content-Type": response.headers.get("Content-Type") ?? "application/json",
      "Cache-Control": "no-store",
    });
    const replayUntil = response.headers.get("Idempotency-Key-Expires-At");
    if (replayUntil) headers.set("Idempotency-Key-Expires-At", replayUntil);
    const requestId = response.headers.get("X-Request-ID");
    if (requestId) headers.set("X-Request-ID", requestId);
    const apiTraceparent = response.headers.get("traceparent");
    if (apiTraceparent) headers.set("traceparent", apiTraceparent);
    return new Response(JSON.stringify(data ?? error), { status: response.status, headers });
  } catch {
    return problem(
      503,
      "Reservation service unavailable",
      process.env.K_SERVICE
        ? "The reservation service could not be reached. Please try again shortly."
        : "The local reservation service could not be reached. Start it and retry.",
      traceparent,
    );
  }
}

export async function DELETE(request: Request) {
  const traceparent = reservationTraceparent(request.headers.get("traceparent"));
  const maxBodyBytes = 512;
  const contentLength = Number(request.headers.get("content-length"));
  if (Number.isFinite(contentLength) && contentLength > maxBodyBytes) {
    return problem(413, "Request too large", "The reservation request is too large.", traceparent);
  }
  const idempotencyKey = request.headers.get("Idempotency-Key") ?? "";
  if (!/^[A-Za-z0-9_-]{22,128}$/.test(idempotencyKey)) {
    return problem(422, "Invalid idempotency key", "Please reload the reservation and try again.", traceparent);
  }

  let payload: unknown;
  try {
    const reader = request.body?.getReader();
    if (!reader) {
      return problem(400, "Invalid request", "The reservation details could not be read.", traceparent);
    }
    const chunks: Uint8Array[] = [];
    let totalBytes = 0;
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      totalBytes += value.byteLength;
      if (totalBytes > maxBodyBytes) {
        await reader.cancel();
        return problem(413, "Request too large", "The reservation request is too large.", traceparent);
      }
      chunks.push(value);
    }
    const body = new Uint8Array(totalBytes);
    let offset = 0;
    for (const chunk of chunks) {
      body.set(chunk, offset);
      offset += chunk.byteLength;
    }
    payload = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(body));
  } catch {
    return problem(400, "Invalid request", "The reservation details could not be read.", traceparent);
  }
  if (!payload || typeof payload !== "object") {
    return problem(422, "Invalid request", "The reservation could not be identified.", traceparent);
  }
  const values = payload as Record<string, unknown>;
  const { event_id: eventId, reservation_id: reservationId } = values;
  if (
    Object.keys(values).length !== 2 ||
    typeof eventId !== "string" || !/^zeno_[A-Za-z0-9]{16,32}$/.test(eventId) ||
    typeof reservationId !== "string" || !/^zeno_[A-Za-z0-9]{16,32}$/.test(reservationId)
  ) {
    return problem(422, "Invalid request", "The reservation could not be identified.", traceparent);
  }

  try {
    const client = createServerApiClient();
    const { response, data, error } = await client.DELETE(
      "/v1/events/{event_id}/reservations/{reservation_id}",
      {
        params: {
          path: { event_id: eventId, reservation_id: reservationId },
          header: { "Idempotency-Key": idempotencyKey, traceparent },
        },
        signal: AbortSignal.timeout(8000),
      },
    );
    const headers = new Headers({
      "Content-Type": response.headers.get("Content-Type") ?? "application/json",
      "Cache-Control": "no-store",
    });
    const requestId = response.headers.get("X-Request-ID");
    if (requestId) headers.set("X-Request-ID", requestId);
    const apiTraceparent = response.headers.get("traceparent");
    if (apiTraceparent) headers.set("traceparent", apiTraceparent);
    return new Response(JSON.stringify(data ?? error), { status: response.status, headers });
  } catch {
    return problem(
      503,
      "Reservation service unavailable",
      process.env.K_SERVICE
        ? "The reservation service could not be reached. Please try again shortly."
        : "The local reservation service could not be reached. Start it and retry.",
      traceparent,
    );
  }
}

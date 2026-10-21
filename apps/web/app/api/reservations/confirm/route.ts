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

type Answer = { field_key: string; value: string };

/**
 * Confirm a held reservation into an order.
 *
 * Answers are passed through rather than interpreted here. The API validates them
 * against the published form and is the only place that knows what a valid answer
 * is; validating twice would let the two disagree.
 */
export async function POST(request: Request) {
  const traceparent = reservationTraceparent(request.headers.get("traceparent"));
  const maxBodyBytes = 16 * 1024;
  const contentLength = Number(request.headers.get("content-length"));
  if (Number.isFinite(contentLength) && contentLength > maxBodyBytes) {
    return problem(413, "Request too large", "The booking details are too large.", traceparent);
  }

  let payload: unknown;
  try {
    const raw = await request.text();
    if (new TextEncoder().encode(raw).byteLength > maxBodyBytes) {
      return problem(413, "Request too large", "The booking details are too large.", traceparent);
    }
    payload = JSON.parse(raw);
  } catch {
    return problem(400, "Invalid request", "The booking details could not be read.", traceparent);
  }

  if (typeof payload !== "object" || payload === null) {
    return problem(422, "Invalid request", "Your booking details are missing.", traceparent);
  }
  const values = payload as Record<string, unknown>;
  const eventId = values.event_id;
  const reservationId = values.reservation_id;
  const purchaserName = values.purchaser_name;
  const purchaserEmail = values.purchaser_email;
  const answers = values.answers ?? [];

  const validAnswers: Answer[] =
    Array.isArray(answers) &&
    answers.every(
      (answer) =>
        typeof answer === "object" &&
        answer !== null &&
        typeof (answer as Answer).field_key === "string" &&
        typeof (answer as Answer).value === "string",
    )
      ? (answers as Answer[]).slice(0, 50)
      : [];

  if (
    typeof eventId !== "string" ||
    !/^zeno_[A-Za-z0-9]{16,32}$/.test(eventId) ||
    typeof reservationId !== "string" ||
    !/^zeno_[A-Za-z0-9]{16,32}$/.test(reservationId) ||
    typeof purchaserName !== "string" ||
    purchaserName.trim().length === 0 ||
    purchaserName.length > 120 ||
    typeof purchaserEmail !== "string" ||
    purchaserEmail.length > 254 ||
    !Array.isArray(answers) ||
    validAnswers.length !== answers.length
  ) {
    return problem(422, "Invalid request", "Check your name, email, and answers.", traceparent);
  }

  try {
    const client = createServerApiClient();
    const { response, data, error } = await client.POST(
      "/v1/events/{event_id}/reservations/{reservation_id}/confirm",
      {
        params: {
          path: { event_id: eventId, reservation_id: reservationId },
        },
        body: {
          purchaser_name: purchaserName.trim(),
          purchaser_email: purchaserEmail.trim(),
          answers: validAnswers,
        },
        signal: AbortSignal.timeout(10_000),
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
      "Booking unavailable",
      process.env.K_SERVICE
        ? "We couldn't complete the booking. Please try again."
        : "The booking service could not be reached. Start it and retry.",
      traceparent,
    );
  }
}
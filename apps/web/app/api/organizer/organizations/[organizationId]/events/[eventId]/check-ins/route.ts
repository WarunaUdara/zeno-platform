import { NextResponse } from "next/server";
import { getAuthenticatedOrganizerApi } from "@/lib/organizer-service";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Proxy a door scan to the API.
 *
 * The client's scanner_action_id is forwarded unchanged so that retrying the same
 * request replays the original outcome instead of admitting a guest twice. That
 * is what makes the door safe over venue Wi-Fi that drops mid-request.
 */
export async function POST(
  request: Request,
  { params }: RouteContext<"/api/organizer/organizations/[organizationId]/events/[eventId]/check-ins">,
) {
  const { organizationId, eventId } = await params;
  if (
    !/^zeno_[A-Za-z0-9]{16,32}$/.test(organizationId) ||
    !/^zeno_[A-Za-z0-9]{16,32}$/.test(eventId)
  ) {
    return NextResponse.json({ title: "Not found" }, { status: 404 });
  }

  let body: unknown;
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ title: "Invalid request" }, { status: 422 });
  }
  const payload = body as { codes?: unknown; scanner_action_id?: unknown };
  if (
    !Array.isArray(payload.codes) ||
    payload.codes.length === 0 ||
    payload.codes.length > 50 ||
    !payload.codes.every((code) => typeof code === "string") ||
    typeof payload.scanner_action_id !== "string" ||
    !/^[A-Za-z0-9_-]{8,64}$/.test(payload.scanner_action_id)
  ) {
    return NextResponse.json({ title: "Invalid request" }, { status: 422 });
  }

  const access = await getAuthenticatedOrganizerApi();
  if (access.state !== "ready") {
    return NextResponse.json(
      { title: "Check-in unavailable" },
      { status: access.state === "signed-out" ? 401 : 503 },
    );
  }

  try {
    const { response, data, error } = await access.value.api.POST(
      "/v1/organizations/{organization_id}/events/{event_id}/check-ins",
      {
        params: { path: { organization_id: organizationId, event_id: eventId } },
        body: {
          codes: payload.codes as string[],
          scanner_action_id: payload.scanner_action_id,
        },
      },
    );
    const requestId = response.headers.get("X-Request-ID");
    return NextResponse.json(data ?? error ?? {}, {
      status: response.status,
      headers: {
        "Cache-Control": "no-store",
        ...(requestId ? { "X-Request-ID": requestId } : {}),
      },
    });
  } catch {
    return NextResponse.json({ title: "Check-in unavailable" }, { status: 503 });
  }
}

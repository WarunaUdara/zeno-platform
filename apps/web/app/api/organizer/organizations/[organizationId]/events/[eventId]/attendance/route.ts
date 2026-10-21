import { NextResponse } from "next/server";
import { getAuthenticatedOrganizerApi } from "@/lib/organizer-service";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

/**
 * Arrival counts for the door counter.
 *
 * Deliberately not cached: a stale "inside" number at a live event is worse than
 * a slower one, and the counter is the thing a volunteer glances at.
 */
export async function GET(
  _request: Request,
  { params }: RouteContext<"/api/organizer/organizations/[organizationId]/events/[eventId]/attendance">,
) {
  const { organizationId, eventId } = await params;
  const access = await getAuthenticatedOrganizerApi();
  if (access.state !== "ready") {
    return NextResponse.json(
      { title: "Attendance unavailable" },
      { status: access.state === "signed-out" ? 401 : 503 },
    );
  }
  try {
    const { response, data, error } = await access.value.api.GET(
      "/v1/organizations/{organization_id}/events/{event_id}/attendance",
      { params: { path: { organization_id: organizationId, event_id: eventId } } },
    );
    return NextResponse.json(data ?? error ?? {}, {
      status: response.status,
      headers: { "Cache-Control": "no-store" },
    });
  } catch {
    return NextResponse.json({ title: "Attendance unavailable" }, { status: 503 });
  }
}
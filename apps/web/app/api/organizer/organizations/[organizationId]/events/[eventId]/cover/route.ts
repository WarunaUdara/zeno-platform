import { getAuthenticatedOrganizerApi } from "@/lib/organizer-service";
import { getNeonObject } from "@/lib/neon-storage";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

function imageTypeForKey(key: string, organizationId: string, eventId: string) {
  const [root, storedOrganizationId, storedEventId, filename, extra] = key.split("/");
  if (
    root !== "events" ||
    storedOrganizationId !== organizationId ||
    storedEventId !== eventId ||
    extra !== undefined ||
    !/^[0-9A-HJKMNP-TV-Z]{26}\.(jpg|png|webp)$/.test(filename ?? "")
  ) return null;
  return filename.endsWith(".jpg")
    ? "image/jpeg"
    : filename.endsWith(".png")
      ? "image/png"
      : "image/webp";
}

export async function GET(
  _request: Request,
  { params }: RouteContext<"/api/organizer/organizations/[organizationId]/events/[eventId]/cover">,
) {
  const { organizationId, eventId } = await params;
  if (!/^zeno_[A-Za-z0-9]{16,32}$/.test(organizationId) || !/^zeno_[A-Za-z0-9]{16,32}$/.test(eventId)) {
    return new Response(null, { status: 404, headers: { "Cache-Control": "no-store" } });
  }

  const access = await getAuthenticatedOrganizerApi();
  if (access.state !== "ready") {
    return new Response(null, { status: access.state === "signed-out" ? 401 : 503, headers: { "Cache-Control": "no-store" } });
  }

  try {
    const event = await access.value.api.GET(
      "/v1/organizations/{organization_id}/events/{event_id}",
      { params: { path: { organization_id: organizationId, event_id: eventId } } },
    );
    if (event.response.status === 404) return new Response(null, { status: 404, headers: { "Cache-Control": "no-store" } });
    if (event.error || !event.data) return new Response(null, { status: 503, headers: { "Cache-Control": "no-store" } });
    if (!event.data.cover_image_key) return new Response(null, { status: 404, headers: { "Cache-Control": "no-store" } });
    const contentType = imageTypeForKey(event.data.cover_image_key, organizationId, eventId);
    if (!contentType) return new Response(null, { status: 404, headers: { "Cache-Control": "no-store" } });

    const image = await getNeonObject(event.data.cover_image_key);
    if (!image.ok || !image.body) return new Response(null, { status: image.status === 404 ? 404 : 502, headers: { "Cache-Control": "no-store" } });
    return new Response(image.body, {
      headers: {
        "Cache-Control": "private, no-store",
        "Content-Type": contentType,
        "X-Content-Type-Options": "nosniff",
        Vary: "Cookie",
      },
    });
  } catch {
    return new Response(null, { status: 503, headers: { "Cache-Control": "no-store" } });
  }
}

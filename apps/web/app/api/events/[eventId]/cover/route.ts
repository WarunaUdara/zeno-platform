import { createServerApiClient } from "@/lib/zeno-api";
import { getNeonObject } from "@/lib/neon-storage";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";

function imageTypeForKey(key: string, eventId: string) {
  const [root, organizationId, storedEventId, filename, extra] = key.split("/");
  if (
    root !== "events" ||
    !/^zeno_[A-Za-z0-9]{16,32}$/.test(organizationId ?? "") ||
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
  { params }: RouteContext<"/api/events/[eventId]/cover">,
) {
  const { eventId } = await params;
  if (!/^zeno_[A-Za-z0-9]{16,32}$/.test(eventId)) return new Response(null, { status: 404 });

  try {
    const event = await createServerApiClient().GET("/v1/events/{event_id}", {
      params: { path: { event_id: eventId } },
    });
    if (event.response.status === 404) return new Response(null, { status: 404 });
    if (event.error || !event.data) return new Response(null, { status: 503, headers: { "Cache-Control": "no-store" } });
    if (!event.data.cover_image_key) return new Response(null, { status: 404 });
    const contentType = imageTypeForKey(event.data.cover_image_key, eventId);
    if (!contentType) return new Response(null, { status: 404 });

    const image = await getNeonObject(event.data.cover_image_key);
    if (!image.ok || !image.body) return new Response(null, { status: image.status === 404 ? 404 : 502 });
    return new Response(image.body, {
      headers: {
        "Cache-Control": "public, max-age=0, s-maxage=60, stale-while-revalidate=60",
        "Content-Type": contentType,
        "X-Content-Type-Options": "nosniff",
      },
    });
  } catch {
    return new Response(null, { status: 503, headers: { "Cache-Control": "no-store" } });
  }
}

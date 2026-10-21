import { localDateTimeToIso } from "@/lib/local-datetime";

export type EventCreateRetry = { fingerprint: string; slug: string };
type RetryStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

const RETRY_KEY_PREFIX = "zeno:event-create:";
const SLUG_PATTERN = /^[a-z0-9](?:[a-z0-9-]{1,78}[a-z0-9])$/;

export async function eventCreateFingerprint(organizationId: string, formData: FormData) {
  const value = (field: string) => {
    const item = formData.get(field);
    return typeof item === "string" ? item.trim() : "";
  };
  const timezone = value("timezone");
  const startsAt = value("starts_at");
  const endsAt = value("ends_at");
  const venueKind = value("venue_kind");
  const normalized = [
    organizationId,
    value("title"),
    value("summary") || null,
    timezone,
    localDateTimeToIso(startsAt, timezone) ?? [startsAt, timezone],
    endsAt ? localDateTimeToIso(endsAt, timezone) ?? [endsAt, timezone] : null,
    venueKind,
    venueKind === "physical" ? value("venue_name") : null,
    venueKind === "physical" ? value("city") : null,
    venueKind === "physical" ? value("country_code").toUpperCase() : null,
  ];
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(JSON.stringify(normalized)));
  return btoa(String.fromCharCode(...new Uint8Array(digest)))
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replace(/=+$/g, "");
}

export function loadEventCreateRetry(
  storage: RetryStorage | null,
  organizationId: string,
  fingerprint: string,
): EventCreateRetry | null {
  if (!storage) return null;
  try {
    const saved = storage.getItem(`${RETRY_KEY_PREFIX}${organizationId}`);
    if (!saved) return null;
    const value: unknown = JSON.parse(saved);
    if (!value || typeof value !== "object") return null;
    const retry = value as Partial<EventCreateRetry>;
    return retry.fingerprint === fingerprint
      && typeof retry.slug === "string"
      && SLUG_PATTERN.test(retry.slug)
      ? { fingerprint, slug: retry.slug }
      : null;
  } catch {
    return null;
  }
}

export function saveEventCreateRetry(
  storage: RetryStorage | null,
  organizationId: string,
  retry: EventCreateRetry,
) {
  if (!storage) return;
  try {
    storage.setItem(`${RETRY_KEY_PREFIX}${organizationId}`, JSON.stringify(retry));
  } catch {
    // The form keeps its retry key in memory if browser storage is unavailable.
  }
}

export function clearEventCreateRetry(storage: RetryStorage | null, organizationId: string) {
  if (!storage) return;
  try {
    storage.removeItem(`${RETRY_KEY_PREFIX}${organizationId}`);
  } catch {
    // Storage cleanup must not block successful event creation.
  }
}

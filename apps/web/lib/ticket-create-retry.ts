import { localDateTimeToIso } from "@/lib/local-datetime";

const TICKET_FIELDS = ["name", "description", "amount", "currency", "capacity", "timezone", "sale_starts_at", "sale_ends_at"] as const;

export async function ticketCreateFingerprint(
  eventId: string,
  organizationId: string,
  formData: FormData,
) {
  const values = Object.fromEntries(TICKET_FIELDS.map((field) => {
    const value = formData.get(field);
    return [field, typeof value === "string" ? value.trim() : ""];
  }));
  const start = localDateTimeToIso(values.sale_starts_at, values.timezone);
  const end = localDateTimeToIso(values.sale_ends_at, values.timezone);
  const normalized = [
    eventId,
    organizationId,
    values.name,
    values.description || null,
    values.amount,
    values.currency.toUpperCase(),
    Number(values.capacity),
    start ?? [values.sale_starts_at, values.timezone],
    end ?? [values.sale_ends_at, values.timezone],
  ];
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(JSON.stringify(normalized)));
  return btoa(String.fromCharCode(...new Uint8Array(digest)))
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replace(/=+$/g, "");
}

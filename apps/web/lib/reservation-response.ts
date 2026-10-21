export async function readReservationResponseBody(response: Pick<Response, "json">) {
  try {
    const body: unknown = await response.json();
    if (typeof body !== "object" || body === null || Array.isArray(body)) return null;
    return body as Record<string, unknown>;
  } catch {
    return null;
  }
}

export type ReservationReleaseStatus = "released" | "expired";

export async function readReservationReleaseStatus(
  response: Pick<Response, "ok" | "json">,
): Promise<ReservationReleaseStatus | null> {
  if (!response.ok) return null;

  const body = await readReservationResponseBody(response);
  return body?.status === "released" || body?.status === "expired"
    ? body.status
    : null;
}

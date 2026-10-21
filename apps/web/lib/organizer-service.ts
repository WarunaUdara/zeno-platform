import "server-only";
import { createServerApiClient } from "@/lib/zeno-api";
import { getNeonAuth, isNeonAuthConfigured } from "@/lib/auth/server";
import { classifySessionLookup, classifyTokenLookup } from "@/lib/auth-result-state";
import { isValidOrganizationCursor } from "@/lib/organization-selection";
import type { components } from "@zeno/api-client";

export type Organization = components["schemas"]["Organization"];
export type OrganizerEvent = components["schemas"]["OrganizerEvent"];
export type OrganizerEventSummary = components["schemas"]["OrganizerEventSummary"];
export type OrganizerEventPage = components["schemas"]["OrganizerEventPage"];
export type OrganizerOrganizationPage = components["schemas"]["OrganizationPage"];
export type OrganizerTicket = components["schemas"]["OrganizerTicketType"];

export type OrganizerWorkspace =
  | { state: "auth-not-configured" }
  | { state: "signed-out" }
  | { state: "unavailable" }
  | {
      state: "ready";
      name: string;
      email: string;
      organizations: Organization[];
      organizationCursor: string | null;
      organizationNextCursor: string | null;
    };

export type AuthenticatedOrganizerApi = {
  api: ReturnType<typeof createServerApiClient>;
  name: string;
  email: string;
};

export type OrganizerApiAccess =
  | { state: "auth-not-configured" }
  | { state: "signed-out" }
  | { state: "unavailable" }
  | { state: "ready"; value: AuthenticatedOrganizerApi };

export async function getAuthenticatedOrganizerApi(): Promise<OrganizerApiAccess> {
  if (!isNeonAuthConfigured()) return { state: "auth-not-configured" };

  try {
    const auth = getNeonAuth();
    const sessionResult = await auth.getSession();
    const sessionState = classifySessionLookup(sessionResult);
    if (sessionState.state !== "authenticated") return { state: sessionState.state };

    const tokenResult = classifyTokenLookup(await auth.token());
    if (tokenResult.state !== "ready") return { state: tokenResult.state };

    return {
      state: "ready",
      value: {
        api: createServerApiClient(tokenResult.token),
        name: sessionState.user.name || sessionState.user.email,
        email: sessionState.user.email,
      },
    };
  } catch {
    return { state: "unavailable" };
  }
}

export async function loadOrganizerWorkspace(cursor?: string): Promise<OrganizerWorkspace> {
  if (!isValidOrganizationCursor(cursor)) return { state: "unavailable" };
  const access = await getAuthenticatedOrganizerApi();
  if (access.state !== "ready") return access;
  try {
    const { data, error } = await access.value.api.GET("/v1/organizations", {
      params: { query: { limit: 50, ...(cursor ? { cursor } : {}) } },
    });
    if (error || !data) return { state: "unavailable" };

    return {
      state: "ready",
      name: access.value.name,
      email: access.value.email,
      organizations: data.items,
      organizationCursor: cursor ?? null,
      organizationNextCursor: data.next_cursor,
    };
  } catch {
    return { state: "unavailable" };
  }
}

export type OrganizerAttendance = components["schemas"]["Attendance"];

export type OrganizerAttendanceState =
  | { state: "auth-not-configured" }
  | { state: "signed-out" }
  | { state: "unavailable" }
  | { state: "not-found" }
  | { state: "ready"; value: OrganizerAttendance };

/**
 * Arrival counts for the door counter.
 *
 * Separate from loadOrganizerWorkspace because the chrome needs the workspace
 * role while the counter needs the API client, and threading both through one
 * call would make every organizer page aware of the door.
 */
export async function loadEventAttendance(
  organizationId: string,
  eventId: string,
): Promise<OrganizerAttendanceState> {
  if (
    !isNeonAuthConfigured() ||
    !/^zeno_[A-Za-z0-9]{16,32}$/.test(organizationId) ||
    !/^zeno_[A-Za-z0-9]{16,32}$/.test(eventId)
  ) {
    return { state: "not-found" };
  }
  const access = await getAuthenticatedOrganizerApi();
  if (access.state === "auth-not-configured") return { state: "auth-not-configured" };
  if (access.state === "signed-out") return { state: "signed-out" };
  if (access.state !== "ready") return { state: "unavailable" };
  try {
    const { response, data } = await access.value.api.GET(
      "/v1/organizations/{organization_id}/events/{event_id}/attendance",
      { params: { path: { organization_id: organizationId, event_id: eventId } } },
    );
    if (response.status === 404) return { state: "not-found" };
    // A failed counter must not block the door. Showing zeros is worse than a
    // slightly stale count, but neither may stop a volunteer admitting guests.
    return { state: "ready", value: data ?? { event_id: eventId, issued: 0, admitted: 0, awaiting: 0, voided: 0 } };
  } catch {
    return { state: "unavailable" };
  }
}

export async function loadOrganizerEvent(organizationId: string, eventId: string) {
  if (!isNeonAuthConfigured() || !/^zeno_[A-Za-z0-9]{16,32}$/.test(organizationId) || !/^zeno_[A-Za-z0-9]{16,32}$/.test(eventId)) {
    return { state: "not-found" as const };
  }

  const access = await getAuthenticatedOrganizerApi();
  if (access.state === "auth-not-configured") return { state: "auth-not-configured" as const };
  if (access.state === "signed-out") return { state: "signed-out" as const };
  if (access.state !== "ready") return { state: "unavailable" as const };

  try {
    const [{ data: event, error: eventError }, { data: ticketPage, error: ticketError }] = await Promise.all([
      access.value.api.GET("/v1/organizations/{organization_id}/events/{event_id}", {
        params: { path: { organization_id: organizationId, event_id: eventId } },
      }),
      access.value.api.GET("/v1/organizations/{organization_id}/events/{event_id}/ticket-types", {
        params: { path: { organization_id: organizationId, event_id: eventId } },
      }),
    ]);
    if (eventError || ticketError || !event || !ticketPage) {
      const eventStatus = eventError ? eventError.status : undefined;
      const ticketStatus = ticketError ? ticketError.status : undefined;
      if (eventStatus === 404 || ticketStatus === 404) return { state: "not-found" as const };
      return { state: "unavailable" as const };
    }

    return { state: "ready" as const, event, tickets: ticketPage.items };
  } catch {
    return { state: "unavailable" as const };
  }
}

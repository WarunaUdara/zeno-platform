"use server";

import { revalidatePath } from "next/cache";
import {
  getAuthenticatedOrganizerApi,
  type OrganizerEventPage,
  type OrganizerOrganizationPage,
} from "@/lib/organizer-service";
import { localDateTimeToIso } from "@/lib/local-datetime";
import { isValidOrganizationCursor } from "@/lib/organization-selection";
import { organizerFormError, organizerListError } from "@/lib/organizer-errors";
import { isValidWorkspaceSlug } from "@/lib/workspace-slug";
import { deleteNeonObject, putNeonObject } from "@/lib/neon-storage";
import { detectCoverImageContentType, MAX_COVER_IMAGE_BYTES } from "@/lib/cover-image-validation";
import { normalizeCoverImage } from "@/lib/cover-image-processing";

type ActionResult =
  | { ok: true; organizationId?: string; eventId?: string }
  | { ok: false; message: string; field?: string };

type OrganizerEventPageResult =
  | { ok: true; page: OrganizerEventPage }
  | { ok: false; message: string };

type OrganizerOrganizationPageResult =
  | { ok: true; page: OrganizerOrganizationPage; cursor: string | null }
  | { ok: false; message: string };

function text(formData: FormData, name: string, maximum = 2000) {
  const value = formData.get(name);
  if (typeof value !== "string" || value.length > maximum) return null;
  return value.trim();
}

function optionalText(formData: FormData, name: string, maximum: number) {
  const value = formData.get(name);
  if (value === null) return { valid: true, value: null as string | null };
  if (typeof value !== "string" || value.length > maximum) return { valid: false, value: null };
  return { valid: true, value: value.trim() || null };
}

function validPublicId(value: string | null): value is string {
  return !!value && /^zeno_[A-Za-z0-9]{16,32}$/.test(value);
}

async function access() {
  const result = await getAuthenticatedOrganizerApi();
  if (result.state === "ready") return result.value;
  if (result.state === "auth-not-configured") {
    return { error: "Organizer sign-in is not connected for this deployment." } as const;
  }
  if (result.state === "signed-out") return { error: "Your session ended. Sign in again to continue." } as const;
  return { error: "Organizer services are temporarily unavailable. Please try again." } as const;
}

async function apiRequest<T>(request: () => Promise<T>) {
  try {
    return { state: "ready" as const, value: await request() };
  } catch {
    return { state: "unavailable" as const };
  }
}

function unavailableResult(): ActionResult {
  return { ok: false, message: "The organizer service is temporarily unavailable. Your details are still here; try again." };
}

export async function createOrganization(formData: FormData): Promise<ActionResult> {
  const name = text(formData, "name", 120);
  const slug = text(formData, "slug", 62);
  if (!name || name.length < 2) return { ok: false, field: "name", message: "Enter a workspace name with at least 2 characters." };
  if (!slug || !isValidWorkspaceSlug(slug)) {
    return { ok: false, field: "slug", message: "Use 3–62 lowercase letters, numbers, or hyphens." };
  }

  const context = await access();
  if ("error" in context) return { ok: false, message: context.error };
  const request = await apiRequest(() => context.api.POST("/v1/organizations", { body: { name, slug } }));
  if (request.state !== "ready") return unavailableResult();
  const { data, error, response } = request.value;
  if (error || !data) return response.status === 409
    ? { ok: false, field: "slug", message: "That workspace address is taken. Choose another to continue." }
    : { ok: false, message: organizerFormError(response.status, "Could not create this workspace. Please try again.") };
  revalidatePath("/organizer");
  return { ok: true, organizationId: data.id };
}

export async function loadMoreOrganizerEvents(
  organizationId: string,
  cursor: string,
): Promise<OrganizerEventPageResult> {
  if (!validPublicId(organizationId) || !/^[A-Za-z0-9_-]{1,512}$/.test(cursor)) {
    return { ok: false, message: "This event list link is no longer valid. Refresh the page and try again." };
  }
  const context = await access();
  if ("error" in context) return { ok: false, message: context.error };
  const request = await apiRequest(() => context.api.GET(
    "/v1/organizations/{organization_id}/events",
    {
      params: { path: { organization_id: organizationId }, query: { cursor, limit: 50 } },
    },
  ));
  if (request.state !== "ready") {
    return { ok: false, message: "We couldn’t load more events. Your current list is still here; try again." };
  }
  const { data, error, response } = request.value;
  if (error || !data) {
    return { ok: false, message: organizerListError(response.status, "We couldn’t load more events. Your current list is still here; try again.") };
  }
  return { ok: true, page: data };
}

export async function loadOrganizerOrganizationPage(
  cursor: string | null,
): Promise<OrganizerOrganizationPageResult> {
  if (!isValidOrganizationCursor(cursor)) {
    return { ok: false, message: "This workspace list link is no longer valid. Reload the organizer workspace and try again." };
  }
  const context = await access();
  if ("error" in context) return { ok: false, message: context.error };
  const request = await apiRequest(() => context.api.GET("/v1/organizations", {
    params: { query: { limit: 50, ...(cursor ? { cursor } : {}) } },
  }));
  if (request.state !== "ready") {
    return { ok: false, message: "We couldn’t load more workspaces. Try again in a moment." };
  }
  const { data, error, response } = request.value;
  if (error || !data) {
    return { ok: false, message: organizerListError(response.status, "We couldn’t load more workspaces. Try again in a moment.") };
  }
  return { ok: true, page: data, cursor };
}

export async function createDraftEvent(formData: FormData): Promise<ActionResult> {
  const organizationId = text(formData, "organization_id", 64);
  const slug = text(formData, "slug", 80);
  const title = text(formData, "title", 160);
  const summary = text(formData, "summary", 1000) || null;
  const timeZone = text(formData, "timezone", 128);
  const startsAt = localDateTimeToIso(text(formData, "starts_at", 32) ?? "", timeZone ?? "");
  const endsAtInput = text(formData, "ends_at", 32) ?? "";
  const endsAt = endsAtInput ? localDateTimeToIso(endsAtInput, timeZone ?? "") : null;
  const venueKind = text(formData, "venue_kind", 16);
  if (!validPublicId(organizationId)) return { ok: false, message: "Choose a valid workspace." };
  if (!slug || !/^[a-z0-9]([a-z0-9-]{1,78}[a-z0-9])$/.test(slug)) return { ok: false, field: "title", message: "The event title needs a longer public link." };
  if (!title || title.length < 1) return { ok: false, field: "title", message: "Add an event name." };
  if (!timeZone || !startsAt) return { ok: false, field: "starts_at", message: "Choose a valid start time and timezone." };
  if (endsAtInput && !endsAt) return { ok: false, field: "ends_at", message: "Choose a valid end time in this timezone." };
  if (endsAt && endsAt <= startsAt) return { ok: false, field: "ends_at", message: "The end time must be after the start time." };
  if (venueKind !== "online" && venueKind !== "physical") return { ok: false, field: "venue_kind", message: "Choose online or in-person." };

  const venue = venueKind === "online"
    ? { kind: "online" as const }
    : {
        kind: "physical" as const,
        name: text(formData, "venue_name", 160) ?? "",
        city: text(formData, "city", 120) ?? "",
        country_code: (text(formData, "country_code", 2) ?? "").toUpperCase(),
      };
  if (venue.kind === "physical" && (!venue.name || !venue.city || !/^[A-Z]{2}$/.test(venue.country_code))) {
    return { ok: false, field: "venue_name", message: "Add the venue name, city, and a 2-letter country code." };
  }

  const context = await access();
  if ("error" in context) return { ok: false, message: context.error };
  const request = await apiRequest(() => context.api.POST(
    "/v1/organizations/{organization_id}/events",
    {
      params: { path: { organization_id: organizationId } },
      body: { slug, title, summary, starts_at: startsAt, ends_at: endsAt, timezone: timeZone, venue },
    },
  ));
  if (request.state !== "ready") return unavailableResult();
  const { data, error, response } = request.value;
  if (error || !data) return { ok: false, message: organizerFormError(response.status, "Could not save this event yet. Your details are still here; try again.") };
  revalidatePath("/organizer");
  return { ok: true, eventId: data.id };
}

export async function updateDraftEvent(formData: FormData): Promise<ActionResult> {
  const organizationId = text(formData, "organization_id", 64);
  const eventId = text(formData, "event_id", 64);
  const title = text(formData, "title", 160);
  const summary = text(formData, "summary", 1000) || null;
  const timeZone = text(formData, "timezone", 128);
  const startsAt = localDateTimeToIso(text(formData, "starts_at", 32) ?? "", timeZone ?? "");
  const endsAtInput = text(formData, "ends_at", 32) ?? "";
  const endsAt = endsAtInput ? localDateTimeToIso(endsAtInput, timeZone ?? "") : null;
  const venueKind = text(formData, "venue_kind", 16);
  const accessibilityInfo = optionalText(formData, "accessibility_info", 2000);
  const cancellationPolicy = optionalText(formData, "cancellation_policy", 2000);
  const supportEmail = optionalText(formData, "support_email", 254);
  if (!validPublicId(organizationId) || !validPublicId(eventId)) return { ok: false, message: "This event link is not valid." };
  if (!title || !timeZone || !startsAt) return { ok: false, field: "title", message: "Add an event name and a valid start time." };
  if (endsAtInput && !endsAt) return { ok: false, field: "ends_at", message: "Choose a valid end time in this timezone." };
  if (endsAt && endsAt <= startsAt) return { ok: false, field: "ends_at", message: "The end time must be after the start time." };
  if (venueKind !== "online" && venueKind !== "physical") return { ok: false, field: "venue_kind", message: "Choose online or in-person." };
  if (!accessibilityInfo.valid) return { ok: false, field: "accessibility_info", message: "Keep accessibility details under 2,000 characters." };
  if (!cancellationPolicy.valid) return { ok: false, field: "cancellation_policy", message: "Keep cancellation terms under 2,000 characters." };
  if (!supportEmail.valid || (supportEmail.value && !/^[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)+$/.test(supportEmail.value))) {
    return { ok: false, field: "support_email", message: "Enter a valid email address or leave this blank." };
  }

  const venue = venueKind === "online"
    ? { kind: "online" as const }
    : {
        kind: "physical" as const,
        name: text(formData, "venue_name", 160) ?? "",
        city: text(formData, "city", 120) ?? "",
        country_code: (text(formData, "country_code", 2) ?? "").toUpperCase(),
      };
  if (venue.kind === "physical" && (!venue.name || !venue.city || !/^[A-Z]{2}$/.test(venue.country_code))) {
    return { ok: false, field: "venue_name", message: "Add the venue name, city, and a 2-letter country code." };
  }

  const context = await access();
  if ("error" in context) return { ok: false, message: context.error };
  const request = await apiRequest(() => context.api.PUT(
    "/v1/organizations/{organization_id}/events/{event_id}",
    {
      params: { path: { organization_id: organizationId, event_id: eventId } },
      body: {
        title,
        summary,
        starts_at: startsAt,
        ends_at: endsAt,
        timezone: timeZone,
        venue,
        accessibility_info: accessibilityInfo.value,
        cancellation_policy: cancellationPolicy.value,
        support_email: supportEmail.value,
      },
    },
  ));
  if (request.state !== "ready") return unavailableResult();
  const { error, response } = request.value;
  if (error) return { ok: false, message: organizerFormError(response.status, "Could not save this event yet. Your details are still here; try again.") };
  revalidatePath("/organizer");
  revalidatePath(`/organizer/events/${eventId}`);
  return { ok: true, eventId };
}

function isEventCoverKey(key: string, organizationId: string, eventId: string) {
  const prefix = `events/${organizationId}/${eventId}/`;
  const filename = key.startsWith(prefix) ? key.slice(prefix.length) : "";
  return /^[0-9A-HJKMNP-TV-Z]{26}\.(jpg|png|webp)$/.test(filename);
}

export async function saveEventCoverImage(
  organizationId: string,
  eventId: string,
  formData: FormData,
): Promise<ActionResult> {
  if (!validPublicId(organizationId) || !validPublicId(eventId)) {
    return { ok: false, message: "This event link is not valid." };
  }
  const image = formData.get("image");
  const alt = text(formData, "alt", 240);
  if (!(image instanceof File) || image.size === 0 || !alt) {
    return { ok: false, message: "Choose an image and describe it for guests who use a screen reader." };
  }
  if (image.size > MAX_COVER_IMAGE_BYTES) {
    return { ok: false, message: "Choose an image smaller than 5 MB." };
  }

  const bytes = new Uint8Array(await image.arrayBuffer());
  const detectedType = detectCoverImageContentType(bytes);
  if (!detectedType || image.type !== detectedType) {
    return { ok: false, message: "Use a valid JPEG, PNG, or WebP image." };
  }

  const context = await access();
  if ("error" in context) return { ok: false, message: context.error };

  try {
    const current = await context.api.GET(
      "/v1/organizations/{organization_id}/events/{event_id}",
      { params: { path: { organization_id: organizationId, event_id: eventId } } },
    );
    if (current.error || !current.data) {
      return { ok: false, message: organizerFormError(current.response.status, "Could not verify this event before uploading. Try again.") };
    }

    const uploadKey = await context.api.POST(
      "/v1/organizations/{organization_id}/events/{event_id}/cover-uploads",
      {
        params: { path: { organization_id: organizationId, event_id: eventId } },
        body: { content_type: "image/webp" },
      },
    );
    if (uploadKey.error || !uploadKey.data) {
      return { ok: false, message: organizerFormError(uploadKey.response.status, "This event cannot accept an image right now. Refresh and try again.") };
    }

    let normalized: Buffer;
    try {
      normalized = await normalizeCoverImage(bytes);
    } catch {
      return { ok: false, message: "We couldn’t safely read this image. Choose a different JPEG, PNG, or WebP file." };
    }
    if (normalized.byteLength > MAX_COVER_IMAGE_BYTES) {
      return { ok: false, message: "This image is too detailed to store safely. Choose a smaller image." };
    }
    await putNeonObject(uploadKey.data.key, normalized, "image/webp");
    const saved = await context.api.PUT(
      "/v1/organizations/{organization_id}/events/{event_id}/cover-image",
      {
        params: { path: { organization_id: organizationId, event_id: eventId } },
        body: { key: uploadKey.data.key, alt },
      },
    );
    if (saved.error || !saved.data) {
      await deleteNeonObject(uploadKey.data.key).catch(() => {
        console.warn("Unattached event cover remains in private storage for later cleanup.");
      });
      return { ok: false, message: organizerFormError(saved.response.status, "The image uploaded but could not be attached to this event. Try again.") };
    }

    if (
      current.data.cover_image_key &&
      current.data.cover_image_key !== uploadKey.data.key &&
      isEventCoverKey(current.data.cover_image_key, organizationId, eventId)
    ) {
      await deleteNeonObject(current.data.cover_image_key).catch(() => {
        console.warn("Previous event cover could not be removed from private storage.");
      });
    }
    revalidatePath(`/organizer/events/${eventId}`);
    revalidatePath(`/events/${eventId}`);
    return { ok: true, eventId };
  } catch {
    return { ok: false, message: "Image storage is temporarily unavailable. Your existing event image is unchanged; try again." };
  }
}

export async function removeEventCoverImage(
  organizationId: string,
  eventId: string,
): Promise<ActionResult> {
  if (!validPublicId(organizationId) || !validPublicId(eventId)) {
    return { ok: false, message: "This event link is not valid." };
  }
  const context = await access();
  if ("error" in context) return { ok: false, message: context.error };

  try {
    const current = await context.api.GET(
      "/v1/organizations/{organization_id}/events/{event_id}",
      { params: { path: { organization_id: organizationId, event_id: eventId } } },
    );
    if (current.error || !current.data) {
      return { ok: false, message: organizerFormError(current.response.status, "Could not verify this event before removing its image. Try again.") };
    }
    if (!current.data.cover_image_key) return { ok: true, eventId };

    const removed = await context.api.PUT(
      "/v1/organizations/{organization_id}/events/{event_id}/cover-image",
      {
        params: { path: { organization_id: organizationId, event_id: eventId } },
        body: { key: null, alt: null },
      },
    );
    if (removed.error) {
      return { ok: false, message: organizerFormError(removed.response.status, "The image could not be removed. Refresh and try again.") };
    }
    if (isEventCoverKey(current.data.cover_image_key, organizationId, eventId)) {
      await deleteNeonObject(current.data.cover_image_key).catch(() => {
        console.warn("Removed event cover remains in private storage for later cleanup.");
      });
    }
    revalidatePath(`/organizer/events/${eventId}`);
    revalidatePath(`/events/${eventId}`);
    return { ok: true, eventId };
  } catch {
    return { ok: false, message: "Image storage is temporarily unavailable. Try again." };
  }
}

export async function createTicketType(formData: FormData): Promise<ActionResult> {
  const organizationId = text(formData, "organization_id", 64);
  const eventId = text(formData, "event_id", 64);
  const idempotencyKey = text(formData, "idempotency_key", 128);
  const name = text(formData, "name", 120);
  const description = text(formData, "description", 500) || null;
  const amount = text(formData, "amount", 32);
  const currency = (text(formData, "currency", 3) ?? "").toUpperCase();
  const capacity = Number(text(formData, "capacity", 16));
  const timeZone = text(formData, "timezone", 128);
  const saleStartsAt = localDateTimeToIso(text(formData, "sale_starts_at", 32) ?? "", timeZone ?? "");
  const saleEndsAt = localDateTimeToIso(text(formData, "sale_ends_at", 32) ?? "", timeZone ?? "");
  if (!validPublicId(organizationId) || !validPublicId(eventId)) return { ok: false, message: "This event link is not valid." };
  if (!idempotencyKey || idempotencyKey.length < 22 || !/^[A-Za-z0-9_-]+$/.test(idempotencyKey)) return { ok: false, message: "Refresh the page and try adding the ticket again." };
  if (!name || !amount || !/^(0|[1-9][0-9]{0,13})(\.[0-9]{1,4})?$/.test(amount)) return { ok: false, field: "amount", message: "Enter a ticket name and a valid amount, such as 25.00." };
  if (!/^[A-Z]{3}$/.test(currency)) return { ok: false, field: "currency", message: "Use a 3-letter currency code, such as USD." };
  if (!Number.isSafeInteger(capacity) || capacity < 1 || capacity > 2_147_483_647) return { ok: false, field: "capacity", message: "Enter a ticket quantity greater than zero." };
  if (!saleStartsAt || !saleEndsAt || saleEndsAt <= saleStartsAt) return { ok: false, field: "sale_ends_at", message: "Choose a valid sale window; the end must follow the start." };

  const context = await access();
  if ("error" in context) return { ok: false, message: context.error };
  const request = await apiRequest(() => context.api.POST(
    "/v1/organizations/{organization_id}/events/{event_id}/ticket-types",
    {
      params: {
        path: { organization_id: organizationId, event_id: eventId },
        header: { "Idempotency-Key": idempotencyKey },
      },
      body: { name, description, amount, currency, capacity, sale_starts_at: saleStartsAt, sale_ends_at: saleEndsAt },
    },
  ));
  if (request.state !== "ready") return unavailableResult();
  const { error, response } = request.value;
  if (error) return { ok: false, message: organizerFormError(response.status, "Could not add this ticket type yet. Your details are still here; try again.") };
  revalidatePath("/organizer");
  revalidatePath(`/organizer/events/${eventId}`);
  return { ok: true, eventId };
}

export async function updateTicketType(formData: FormData): Promise<ActionResult> {
  const organizationId = text(formData, "organization_id", 64);
  const eventId = text(formData, "event_id", 64);
  const ticketTypeId = text(formData, "ticket_type_id", 64);
  const name = text(formData, "name", 120);
  const description = text(formData, "description", 500) || null;
  const amount = text(formData, "amount", 32);
  const currency = (text(formData, "currency", 3) ?? "").toUpperCase();
  const capacity = Number(text(formData, "capacity", 16));
  const timeZone = text(formData, "timezone", 128);
  const saleStartsAt = localDateTimeToIso(text(formData, "sale_starts_at", 32) ?? "", timeZone ?? "");
  const saleEndsAt = localDateTimeToIso(text(formData, "sale_ends_at", 32) ?? "", timeZone ?? "");
  if (!validPublicId(organizationId) || !validPublicId(eventId) || !validPublicId(ticketTypeId)) return { ok: false, message: "This ticket link is not valid." };
  if (!name || !amount || !/^(0|[1-9][0-9]{0,13})(\.[0-9]{1,4})?$/.test(amount)) return { ok: false, field: "amount", message: "Enter a ticket name and a valid amount, such as 25.00." };
  if (!/^[A-Z]{3}$/.test(currency)) return { ok: false, field: "currency", message: "Use a 3-letter currency code, such as USD." };
  if (!Number.isSafeInteger(capacity) || capacity < 1 || capacity > 2_147_483_647) return { ok: false, field: "capacity", message: "Enter a ticket quantity greater than zero." };
  if (!saleStartsAt || !saleEndsAt || saleEndsAt <= saleStartsAt) return { ok: false, field: "sale_ends_at", message: "Choose a valid sale window; the end must follow the start." };

  const context = await access();
  if ("error" in context) return { ok: false, message: context.error };
  const request = await apiRequest(() => context.api.PUT(
    "/v1/organizations/{organization_id}/events/{event_id}/ticket-types/{ticket_type_id}",
    {
      params: { path: { organization_id: organizationId, event_id: eventId, ticket_type_id: ticketTypeId } },
      body: { name, description, amount, currency, capacity, sale_starts_at: saleStartsAt, sale_ends_at: saleEndsAt },
    },
  ));
  if (request.state !== "ready") return unavailableResult();
  const { error, response } = request.value;
  if (error) return { ok: false, message: organizerFormError(response.status, "Could not save this ticket type. Check that its quantity still covers existing holds and sales.") };
  revalidatePath("/organizer");
  revalidatePath(`/organizer/events/${eventId}`);
  return { ok: true, eventId };
}

export async function publishEvent(formData: FormData): Promise<ActionResult> {
  const organizationId = text(formData, "organization_id", 64);
  const eventId = text(formData, "event_id", 64);
  if (!validPublicId(organizationId) || !validPublicId(eventId)) return { ok: false, message: "This event link is not valid." };

  const context = await access();
  if ("error" in context) return { ok: false, message: context.error };
  const request = await apiRequest(() => context.api.POST(
    "/v1/organizations/{organization_id}/events/{event_id}/publish",
    { params: { path: { organization_id: organizationId, event_id: eventId } } },
  ));
  if (request.state !== "ready") return unavailableResult();
  const { error, response } = request.value;
  if (error) {
    return {
      ok: false,
      message: response.status === 409
        ? "To publish, add a ticket with places remaining and a sale window that hasn’t ended."
        : organizerFormError(response.status, "Could not publish this event yet. Please try again."),
    };
  }
  revalidatePath("/organizer");
  revalidatePath(`/organizer/events/${eventId}`);
  revalidatePath("/events");
  return { ok: true, eventId };
}

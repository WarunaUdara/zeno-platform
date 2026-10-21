"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useRef, useState, useTransition } from "react";
import { authClient } from "@/lib/auth/client";
import { createOrganization, loadMoreOrganizerEvents } from "@/app/organizer/actions";
import { createEventClock } from "@/lib/event-time";
import { canManageEvents, canRunCheckIn } from "@/lib/organizer-permissions";
import { isValidWorkspaceSlug, suggestedWorkspaceSlug } from "@/lib/workspace-slug";
import type { Organization, OrganizerEventSummary } from "@/lib/organizer-service";
import { Button } from "@/components/ui/button";
import styles from "./workspace.module.css";

export function SignOutButton() {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [message, setMessage] = useState("");

  return (
    <div className={styles.signOutWrap}>
      <button
        className={styles.signOut}
        disabled={pending}
        onClick={() => {
          setMessage("");
          startTransition(async () => {
            try {
              const result = await authClient.signOut();
              if (result.error) {
                setMessage("We couldn’t sign you out. Check your connection and try again.");
                return;
              }
              router.replace("/organizer");
              router.refresh();
            } catch {
              setMessage("We couldn’t sign you out. Check your connection and try again.");
            }
          });
        }}
        type="button"
      >
        {pending ? "Signing out…" : "Sign out"}
      </button>
      {message ? <span className={styles.signOutError} role="alert">{message}</span> : null}
    </div>
  );
}

export function CreateOrganizationForm({ name }: { name: string }) {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [workspaceName, setWorkspaceName] = useState("");
  const [workspaceSlug, setWorkspaceSlug] = useState("");
  const [slugEdited, setSlugEdited] = useState(false);
  const [slugEditorOpen, setSlugEditorOpen] = useState(false);
  const [message, setMessage] = useState("");
  const [fieldError, setFieldError] = useState<string | undefined>();
  const slugEditor = useRef<HTMLDetailsElement>(null);
  const slugInput = useRef<HTMLInputElement>(null);
  const suggestedSlug = suggestedWorkspaceSlug(workspaceName);
  const slug = slugEdited ? workspaceSlug : suggestedSlug;

  useEffect(() => {
    if (fieldError === "slug" && slugEditorOpen) slugInput.current?.focus();
  }, [fieldError, slugEditorOpen]);

  function submit(formData: FormData) {
    if (!isValidWorkspaceSlug(slug)) {
      setMessage("Choose a workspace address with 3–62 lowercase letters, numbers, or hyphens.");
      setFieldError("slug");
      setSlugEditorOpen(true);
      return;
    }
    formData.set("slug", slug);
    setMessage("");
    setFieldError(undefined);
    startTransition(async () => {
      const result = await createOrganization(formData);
      if (result.ok) {
        if (result.organizationId) {
          router.replace(`/organizer?organization=${encodeURIComponent(result.organizationId)}`);
          router.refresh();
        }
      } else {
        setMessage(result.message);
        setFieldError(result.field);
        if (result.field === "slug") setSlugEditorOpen(true);
      }
    });
  }

  return (
    <main className={styles.setupMain}>
      <p className={styles.eyebrow}>YOUR FIRST STEP</p>
      <h1>Where do you host events?</h1>
      <p className={styles.intro}>Create a workspace to manage your events, ticket inventory, and reservations.</p>
      <form action={submit} className={styles.workspaceForm}>
        <label htmlFor="workspace-name">Workspace name</label>
        <input
          autoComplete="organization"
          autoFocus
          id="workspace-name"
          maxLength={120}
          name="name"
          onChange={(event) => {
            setWorkspaceName(event.target.value);
            if (fieldError) {
              setFieldError(undefined);
              setMessage("");
            }
          }}
          placeholder="e.g. Campus Arts Society"
          required
          value={workspaceName}
        />
        <input name="slug" type="hidden" value={slug} />
        <details
          className={styles.addressEditor}
          onToggle={(event) => setSlugEditorOpen(event.currentTarget.open)}
          open={slugEditorOpen}
          ref={slugEditor}
        >
          <summary>Customize workspace address <span>Optional</span></summary>
          <label htmlFor="workspace-slug">Workspace address</label>
          <input
            aria-describedby={fieldError === "slug" ? "workspace-address-error" : "workspace-address-hint"}
            aria-invalid={fieldError === "slug" || undefined}
            autoCapitalize="none"
            autoComplete="off"
            id="workspace-slug"
            maxLength={slugEditorOpen ? 62 : undefined}
            minLength={slugEditorOpen ? 3 : undefined}
            onChange={(event) => {
              setSlugEdited(true);
              setWorkspaceSlug(event.target.value.toLowerCase());
              if (fieldError === "slug") {
                setFieldError(undefined);
                setMessage("");
              }
            }}
            pattern={slugEditorOpen ? "[a-z0-9]([a-z0-9-]{1,60}[a-z0-9])" : undefined}
            placeholder="your-team"
            ref={slugInput}
            required={slugEditorOpen}
            value={slug}
          />
          <p className={styles.hint} id="workspace-address-hint">Used to distinguish this workspace. You can keep the suggested address.</p>
        </details>
        {fieldError === "slug" ? <p aria-live="polite" className={styles.formError} id="workspace-address-error" role="alert">{message}</p> : null}
        {fieldError !== "slug" && message ? <p aria-live="polite" className={styles.formError} role="alert">{message}</p> : null}
        <Button disabled={pending || workspaceName.trim().length < 2} size="lg" type="submit">
          {pending ? "Creating workspace…" : "Create workspace"}
        </Button>
      </form>
      <p className={styles.signedInAs}>Signed in as {name}</p>
    </main>
  );
}

export function OrganizerDashboard({
  organization,
  organizationCursor,
  events,
  nextCursor,
}: {
  organization: Organization;
  organizationCursor: string | null;
  events: OrganizerEventSummary[];
  nextCursor: string | null;
}) {
  const [loadedEvents, setLoadedEvents] = useState(() => events);
  const [eventCursor, setEventCursor] = useState(nextCursor);
  const [listMessage, setListMessage] = useState("");
  const [loadingMore, startLoadingMore] = useTransition();
  const mayCreate = canManageEvents(organization.role);
  const mayCheckIn = canRunCheckIn(organization.role);
  const organizationQuery = new URLSearchParams({ organization: organization.id });
  if (organizationCursor) organizationQuery.set("org_cursor", organizationCursor);

  function loadMore() {
    if (!eventCursor) return;
    const requestedCursor = eventCursor;
    setListMessage("");
    startLoadingMore(async () => {
      const result = await loadMoreOrganizerEvents(organization.id, requestedCursor);
      if (!result.ok) {
        setListMessage(result.message);
        return;
      }
      setLoadedEvents((current) => {
        const knownIds = new Set(current.map((event) => event.id));
        return [...current, ...result.page.items.filter((event) => !knownIds.has(event.id))];
      });
      setEventCursor(result.page.next_cursor);
    });
  }

  return (
    <section aria-labelledby="events-heading" className={styles.eventsSection}>
      <div className={styles.titleRow}>
        <div>
          <p className={styles.eyebrow}>EVENTS</p>
          <h1 id="events-heading">Events</h1>
          <p className={styles.intro}>{mayCreate ? `Create and publish events for ${organization.name}.` : `Review event setup for ${organization.name}.`}</p>
        </div>
        {mayCreate ? (
          <Button href={`/organizer/events/new?${organizationQuery.toString()}`} size="lg">
            New event <span aria-hidden="true">+</span>
          </Button>
        ) : null}
      </div>

      {loadedEvents.length ? (
        <>
          <div className={styles.listSummary} aria-live="polite">
            {eventCursor ? "Showing " : ""}{loadedEvents.length} {loadedEvents.length === 1 ? "event" : "events"}
          </div>
          <ul className={styles.eventList}>
            {loadedEvents.map((event) => {
              const clock = createEventClock(event.timezone);
              const date = event.starts_at
                ? clock.format(event.starts_at, { month: "short", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit" })
                : "Date not set";
              const venue = event.venue?.kind === "physical" ? `${event.venue.city}, ${event.venue.country_code}` : "Online";
              const href = `/organizer/events/${encodeURIComponent(event.id)}?${organizationQuery.toString()}`;

              return (
                <li className={styles.eventRow} key={event.id}>
                  <Link aria-label={`${event.title}, ${event.visibility}. ${mayCreate ? "Edit event" : "Review event"}`} className={styles.eventLink} href={href}>
                    <span className={styles.eventCopy}>
                      <span className={styles.eventTitle}>{event.title}</span>
                      <span className={styles.eventMeta}>{date} · {venue} · {event.timezone}</span>
                    </span>
                    <span className={`${styles.status} ${event.visibility === "public" ? styles.statusLive : ""}`}>
                      {event.visibility === "public" ? "Published" : "Draft"}
                    </span>
                    <span aria-hidden="true" className={styles.rowArrow}>→</span>
                  </Link>
                  {event.visibility === "public" ? (
                    <span className={styles.rowLinks}>
                      {mayCheckIn ? (
                        <Link className={styles.publicLink} href={`/organizer/events/${encodeURIComponent(event.id)}/check-in?${organizationQuery.toString()}`}>
                          Run check-in
                        </Link>
                      ) : null}
                      <Link className={styles.publicLink} href={`/events/${encodeURIComponent(event.id)}`}>
                        View public page <span aria-hidden="true">↗</span>
                      </Link>
                    </span>
                  ) : null}
                </li>
              );
            })}
          </ul>
          {eventCursor || listMessage ? (
            <div className={styles.loadMore}>
              {listMessage ? <p aria-live="polite" className={styles.formError} role="alert">{listMessage}</p> : null}
              {eventCursor ? (
                <Button disabled={loadingMore} onClick={loadMore} size="lg" type="button" variant="outline">
                  {loadingMore ? "Loading events…" : "Load more events"}
                </Button>
              ) : null}
            </div>
          ) : null}
        </>
      ) : (
        <div className={styles.emptyState}>
          <p className={styles.emptyTitle}>No events yet</p>
          <p>{mayCreate ? "Create a draft first. It stays private until you publish it." : "There are no events to review yet."}</p>
          {mayCreate ? (
            <Button href={`/organizer/events/new?${organizationQuery.toString()}`} size="lg">
              Create your first event
            </Button>
          ) : <p className={styles.hint}>Ask a workspace owner for event manager access.</p>}
        </div>
      )}
    </section>
  );
}

export function OrganizerUnavailable({ message }: { message: string }) {
  return (
    <main className={styles.setupMain}>
      <p className={styles.eyebrow}>ORGANIZER WORKSPACE</p>
      <h1>We can’t open your workspace right now.</h1>
      <p className={styles.intro}>{message}</p>
      <Button href="/organizer" variant="outline">Try again</Button>
    </main>
  );
}

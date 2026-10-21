import Link from "next/link";
import { connection } from "next/server";
import type { components } from "@zeno/api-client";
import { createServerApiClient } from "@/lib/zeno-api";
import { createEventClock } from "@/lib/event-time";
import styles from "./catalog.module.css";

type EventPage = components["schemas"]["PublishedEventPage"];

function EventCard({ event }: { event: EventPage["items"][number] }) {
  const startsAt = new Date(event.starts_at);
  const clock = createEventClock(event.timezone);
  const date = clock.format(startsAt, {
    weekday: "short",
    month: "short",
    day: "numeric",
    year: "numeric",
  });
  const time = clock.format(startsAt, {
    hour: "numeric",
    minute: "2-digit",
  });
  const month = clock.format(startsAt, { month: "short" });
  const day = clock.format(startsAt, { day: "2-digit" });
  const timezoneLabel = clock.usesUtcFallback ? "UTC (timezone unavailable)" : clock.label;
  const venue = event.venue.kind === "physical"
    ? `${event.venue.name} · ${event.venue.city}`
    : "Online event";

  return (
    <Link className={styles.eventCard} href={`/events/${event.id}`}>
      <div className={styles.dateBlock} aria-label={date}>
        <span>{month}</span>
        <strong>{day}</strong>
      </div>
      <div className={styles.eventCopy}>
        <div className={styles.eventMeta}><span>{date} · {time} · {timezoneLabel}</span></div>
        <h2>{event.title}</h2>
        <p>{event.summary || "Event details and ticket options are available here."}</p>
        <span className={styles.venue}>{venue}</span>
      </div>
      <span aria-hidden="true" className={styles.openArrow}>↗</span>
    </Link>
  );
}

export default async function EventsPage({
  searchParams,
}: {
  searchParams: Promise<{ cursor?: string }>;
}) {
  const { cursor } = await searchParams;
  await connection();
  const client = createServerApiClient();
  let data: EventPage | undefined;
  let catalogUnavailable = false;
  let invalidPageLink = false;
  try {
    const result = await client.GET("/v1/events", {
      params: { query: { limit: 50, ...(cursor ? { cursor } : {}) } },
    });
    data = result.data as EventPage | undefined;
    invalidPageLink = Boolean(cursor) && result.response.status === 422;
    catalogUnavailable = !data && !invalidPageLink;
  } catch {
    catalogUnavailable = true;
  }
  const events = data?.items ?? [];

  return (
    <main className={styles.page}>
      <header className={styles.header}>
        <Link className={styles.brand} href="/">zeno<span>.</span></Link>
        <Link className={styles.backLink} href="/organizer">For organizers <span aria-hidden="true">↗</span></Link>
      </header>
      <section className={styles.intro}>
        <p className={styles.eyebrow}>DISCOVER</p>
        <h1>Find your next<br />good gathering.</h1>
        <p className={styles.description}>Explore upcoming events and choose the experience that fits you.</p>
      </section>
      <section aria-labelledby="upcoming-heading" className={styles.listing}>
        <div className={styles.listHeading}>
          <h2 id="upcoming-heading">Upcoming events</h2>
          <span>
            {catalogUnavailable
              ? "Catalog unavailable"
              : invalidPageLink
                ? "Page link needs updating"
              : `${events.length} ${events.length === 1 ? "event" : "events"} on this page${data?.next_cursor ? " · more available" : ""}`}
          </span>
        </div>
        {invalidPageLink ? (
          <div className={styles.emptyState} role="status">
            <h3>This event list link is no longer valid</h3>
            <p>The catalog changed or this link came from an older page. Start from the current event list.</p>
            <Link className={styles.retryLink} href="/events">Browse upcoming events</Link>
          </div>
        ) : catalogUnavailable ? (
          <div className={styles.emptyState} role="status">
            <h3>Events are temporarily unavailable</h3>
            <p>Try again in a moment, or come back later.</p>
            <Link className={styles.retryLink} href="/events">Try again</Link>
          </div>
        ) : events.length ? (
          <>
            <div className={styles.eventList}>{events.map((event) => <EventCard event={event} key={event.id} />)}</div>
            {data?.next_cursor && (
              <Link className={styles.nextPage} href={`/events?cursor=${encodeURIComponent(data.next_cursor)}`}>
                Show more events <span aria-hidden="true">↓</span>
              </Link>
            )}
          </>
        ) : (
          <div className={styles.emptyState}>
            <h3>No upcoming events yet</h3>
            <p>Published events will appear here as soon as they are ready for guests.</p>
          </div>
        )}
      </section>
    </main>
  );
}

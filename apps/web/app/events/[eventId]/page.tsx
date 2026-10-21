import type { components } from "@zeno/api-client";
import { connection } from "next/server";
import { notFound } from "next/navigation";
import Link from "next/link";
import Image from "next/image";
import { createServerApiClient } from "@/lib/zeno-api";
import { createEventClock } from "@/lib/event-time";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { TicketCheckout } from "./ticket-checkout";
import styles from "./event-page.module.css";

type PublishedEvent = components["schemas"]["PublishedEvent"];
type PublishedTicketType = components["schemas"]["PublishedTicketType"];

function EventArtwork({
  title,
  eventMonth,
  eventId,
  coverImageAlt,
}: {
  title: string;
  eventMonth: string;
  eventId: string;
  coverImageAlt: string | null;
}) {
  if (coverImageAlt) {
    return (
      <div className={styles.artwork}>
        <Image
          alt={coverImageAlt}
          className={styles.coverImage}
          fill
          priority
          sizes="(max-width: 680px) 100vw, 46vw"
          src={`/api/events/${encodeURIComponent(eventId)}/cover`}
          unoptimized
        />
      </div>
    );
  }
  return (
    <div aria-label={`Abstract event poster for ${title}`} className={styles.artwork} role="img">
      <div className={styles.artOrb} />
      <div className={styles.artGrid} />
      <div className={styles.artText}>
        <span>{eventMonth.toUpperCase()}</span>
        <strong>{title}</strong>
      </div>
      <span className={styles.artStamp}>Z</span>
    </div>
  );
}

function EventUnavailable({ eventId }: { eventId: string }) {
  return (
    <main className={styles.eventPage}>
      <header className={styles.siteHeader}>
        <Link className={styles.brand} href="/">
          <span aria-hidden="true" className={styles.brandMark}>z</span><span>zeno<span>.</span></span>
        </Link>
        <Link className={styles.hostLink} href="/events">All events <span>↗</span></Link>
      </header>
      <div className={styles.eventContainer}>
        <section className={styles.loadError} role="status">
          <h1>We couldn’t load this event.</h1>
          <p>Try again, or go back to all events.</p>
          <div className={styles.loadActions}>
            <a href={`/events/${eventId}`}>Try loading again</a>
            <Link href="/events">Browse all events</Link>
          </div>
        </section>
      </div>
    </main>
  );
}

function TicketOptions({
  ticketTypes,
  eventTimezone,
  eventId,
  formFields,
}: {
  ticketTypes: PublishedTicketType[] | null;
  eventTimezone: string;
  eventId: string;
  formFields: components["schemas"]["RegistrationFormField"][];
}) {
  return (
    <Card as="section" aria-labelledby="ticket-options-title" className={styles.ticketPanel} id="ticket-options">
      <div className={styles.ticketHeading}>
        <h2 id="ticket-options-title">Choose tickets</h2>
      </div>
        {ticketTypes ? (
          <TicketCheckout
            eventId={eventId}
            eventTimezone={eventTimezone}
            formFields={formFields}
            ticketTypes={ticketTypes}
          />
        ) : (
          <div className={styles.ticketUnavailable} role="status">
            <p>Ticket options couldn’t be loaded.</p>
            <Link href={`/events/${eventId}`}>Reload event</Link>
          </div>
        )}
    </Card>
  );
}

export default async function EventPreviewPage({
  params,
}: {
  params: Promise<{ eventId: string }>;
}) {
  const { eventId } = await params;
  if (!/^zeno_[A-Za-z0-9]{16,32}$/.test(eventId)) notFound();
  await connection();
  const client = createServerApiClient();
  const [eventResult, ticketTypesResult, formResult] = await Promise.allSettled([
    client.GET("/v1/events/{event_id}", { params: { path: { event_id: eventId } } }),
    client.GET("/v1/events/{event_id}/ticket-types", {
      params: { path: { event_id: eventId } },
    }),
    // Most events ask nothing, so a missing form is the normal case and must not
    // slow the page down or turn into an error.
    client.GET("/v1/events/{event_id}/registration-form", {
      params: { path: { event_id: eventId } },
    }),
  ]);
  if (eventResult.status === "rejected") {
    return <EventUnavailable eventId={eventId} />;
  }

  const eventResponse = eventResult.value;
  if (!eventResponse.data) {
    if (eventResponse.response.status === 404) notFound();
    return <EventUnavailable eventId={eventId} />;
  }

  const event: PublishedEvent = eventResponse.data;
  const ticketTypes =
    ticketTypesResult.status === "fulfilled" ? ticketTypesResult.value.data?.items ?? null : null;
  const formFields =
    formResult.status === "fulfilled" ? formResult.value.data?.fields ?? [] : [];
  const eventDate = new Date(event.starts_at);
  const eventClock = createEventClock(event.timezone);
  const month = eventClock.format(eventDate, {
    month: "short",
  });
  const day = eventClock.format(eventDate, {
    day: "2-digit",
  });
  const dateLabel = eventClock.format(eventDate, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });
  const artworkMonth = eventClock.format(eventDate, {
    month: "short",
    year: "numeric",
  });
  const startTime = eventClock.format(eventDate, {
    hour: "numeric",
    minute: "2-digit",
  });
  const endTime = event.ends_at
    ? eventClock.format(new Date(event.ends_at), {
        hour: "numeric",
        minute: "2-digit",
      })
    : null;
  const timezoneLabel = eventClock.usesUtcFallback
    ? "UTC (event timezone unavailable)"
    : `${eventClock.label} time`;
  const venueName = event.venue.kind === "physical" ? event.venue.name : "Online event";
  const venueLocation =
    event.venue.kind === "physical"
      ? `${event.venue.city}, ${event.venue.country_code}`
      : "Join from anywhere";

  return (
    <main className={styles.eventPage}>
      <header className={styles.siteHeader}>
        <Link className={styles.brand} href="/">
          <span aria-hidden="true" className={styles.brandMark}>z</span><span>zeno<span>.</span></span>
        </Link>
        <Link className={styles.hostLink} href="/events">All events <span>↗</span></Link>
      </header>

      <div className={styles.eventContainer}>
        <div className={styles.eventBreadcrumb}><span>EVENTS</span><i>/</i><strong>{event.title.toUpperCase()}</strong></div>
        <section className={styles.hero}>
          <EventArtwork coverImageAlt={event.cover_image_alt} eventId={event.id} eventMonth={artworkMonth} title={event.title} />
          <div className={styles.heroCopy}>
            <h1>{event.title}</h1>
            <p className={styles.heroDescription}>{event.summary ?? "The organizer hasn’t added an event summary yet."}</p>
            <div className={styles.heroMeta}>
              <span><b>{month}</b><strong>{day}</strong></span>
              <span><strong>{dateLabel}</strong><small>{startTime}{endTime ? ` – ${endTime}` : ""} · {timezoneLabel}</small></span>
              <span className={styles.metaDivider} />
              <span><strong>{venueName}</strong><small>{venueLocation}</small></span>
            </div>
            <Button className={styles.heroCta} href="#ticket-options" size="lg">Choose tickets <span>↓</span></Button>
          </div>
        </section>

        <div className={styles.eventGrid}>
          <div className={styles.eventStory}>
            <section className={styles.storySection}>
              <p className={styles.eyebrow}>PLAN YOUR VISIT</p>
              <h2>Before you go</h2>
              {event.accessibility_info || event.cancellation_policy || event.support_email ? (
                // Only render rows the organizer actually filled in. A list padded
                // with "not provided" tells a guest nothing and makes the page
                // look unfinished, so absent details are simply absent.
                <dl className={styles.attendeeDetails}>
                  {event.accessibility_info ? (
                    <div><dt>Accessibility and access</dt><dd>{event.accessibility_info}</dd></div>
                  ) : null}
                  {event.cancellation_policy ? (
                    <div><dt>Cancellation and transfer</dt><dd>{event.cancellation_policy}</dd></div>
                  ) : null}
                  {event.support_email ? (
                    <div><dt>Questions</dt><dd><a href={`mailto:${event.support_email}`}>{event.support_email}</a></dd></div>
                  ) : null}
                </dl>
              ) : (
                <p className={styles.detailsIntro}>The organizer hasn&apos;t added access, cancellation, or contact details yet.</p>
              )}
            </section>
            <section aria-label="Reservation terms" className={styles.reservationTerms}>
              <p className={styles.eyebrow}>RESERVATION TERMS</p>
              <p>Holds last 15 minutes. No payment is collected. Entry tickets are issued when you confirm.</p>
            </section>
          </div>
          <aside className={styles.eventAside}>
            <TicketOptions
              eventId={event.id}
              eventTimezone={event.timezone}
              formFields={formFields}
              ticketTypes={ticketTypes}
            />
          </aside>
        </div>

      </div>
    </main>
  );
}

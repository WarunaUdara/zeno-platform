import Link from "next/link";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Icon } from "@/components/overview/icons";
import { getOverviewEventPreview } from "@/lib/overview-data";
import { createEventClock } from "@/lib/event-time";
import type { EventPreviewState } from "@/lib/overview-data";
import styles from "./landing-page.module.css";

const steps = [
  {
    number: "01",
    title: "Publish your event",
    description: "Share the date, venue, and ticket options your guests need.",
  },
  {
    number: "02",
    title: "Tickets stay reserved",
    description: "Each guest gets 15 minutes to finish, so nobody can take their spots.",
  },
  {
    number: "03",
    title: "Nothing gets stranded",
    description: "Spots from abandoned checkouts return to sale on their own.",
  },
];

function EventPreview({ preview }: { preview: EventPreviewState }) {
  if (preview.state !== "available") {
    const unavailable = preview.state === "unavailable";
    return (
      <section aria-label="Public event preview" className={`${styles.preview} ${styles.previewEmpty}`}>
        <div className={styles.previewBar}>
          <span className={styles.previewBrand}>zeno<span>.</span></span>
          <Badge variant="outline">ATTENDEE EXPERIENCE</Badge>
        </div>
        <div className={styles.emptyPreview} role="status">
          <Icon name="calendar" size={22} />
          <p>{unavailable ? "Event preview is temporarily unavailable." : "No published demo event yet."}</p>
          <Link href="/events">{unavailable ? "Check the event catalog" : "Open the attendee demo"} <Icon name="arrow-up-right" size={15} /></Link>
        </div>
      </section>
    );
  }

  const event = preview.page.items[0];
  const clock = createEventClock(event.timezone);
  const date = clock.format(event.starts_at, { weekday: "short", month: "short", day: "numeric" });
  const time = clock.format(event.starts_at, { hour: "numeric", minute: "2-digit" });
  const timezone = clock.usesUtcFallback ? "UTC" : clock.label;
  const venue = event.venue.kind === "physical" ? `${event.venue.name} · ${event.venue.city}` : "Online event";

  return (
    <section aria-label={`Preview of ${event.title}`} className={styles.preview}>
      <div className={styles.previewBar}>
        <span className={styles.previewBrand}>zeno<span>.</span></span>
        <Badge variant="outline">ATTENDEE EXPERIENCE</Badge>
      </div>
      <Link aria-label={`Open event preview: ${event.title}`} className={styles.eventPreview} href={`/events/${encodeURIComponent(event.id)}`}>
        <span className={styles.previewEyebrow}>UPCOMING EVENT</span>
        <span className={styles.eventDate}>{date}<span>·</span>{time}<span>·</span>{timezone}</span>
        <span className={styles.eventTitle}>{event.title}</span>
        <span className={styles.eventSummary}>{event.summary || "Event details and ticket options are available here."}</span>
        <span className={styles.eventVenue}><Icon name="calendar" size={16} />{venue}</span>
        <span className={styles.eventOpen}>View event <Icon name="arrow-up-right" size={15} /></span>
      </Link>
      <div className={styles.previewFoot}>
        <span>Published event page</span><span>Reservation demo · no payment</span>
      </div>
    </section>
  );
}

export async function LandingPage() {
  const eventPreview = await getOverviewEventPreview();

  return (
    <main className={styles.page}>
      <header className={styles.header}>
        <Link aria-label="Zeno home" className={styles.brand} href="/">
          <span aria-hidden="true" className={styles.brandMark}>z</span><span>zeno<span>.</span></span>
        </Link>
        <nav aria-label="Main navigation" className={styles.nav}>
          <Link href="/events">Browse events</Link>
          <Link href="#flow-title">How it works</Link>
          <Button href="/organizer" size="default" variant="outline">Host an event</Button>
        </nav>
      </header>

      <section aria-labelledby="landing-title" className={styles.hero}>
        <div className={styles.heroCopy}>
          <p className={styles.eyebrow}>FOR EVENT ORGANIZERS</p>
          <h1 id="landing-title">Publish your event. Protect its capacity.</h1>
          <p className={styles.lede}>Share your event details, hold tickets for 15 minutes while guests check out, and return abandoned spots to sale automatically.</p>
          <div className={styles.heroActions}>
            <Button href="/events" size="lg">Browse events <Icon name="arrow-right" size={16} /></Button>
            <Link className={styles.secondaryLink} href="#flow-title">How reservations work <Icon name="arrow-down-right" size={15} /></Link>
          </div>
          <p className={styles.boundary}>Spots are never oversold. No payment is taken; confirming issues one entry ticket per guest, each with its own QR code.</p>
        </div>
        <EventPreview preview={eventPreview} />
      </section>

      <section aria-labelledby="flow-title" className={styles.flow}>
        <div className={styles.flowHeading}>
          <p className={styles.eyebrow}>THE CORE FLOW</p>
          <h2 id="flow-title">From event page to available inventory.</h2>
        </div>
        <ol className={styles.steps}>
          {steps.map((step) => (
            <li key={step.number}>
              <span className={styles.stepNumber}>{step.number}</span>
              <div><h3>{step.title}</h3><p>{step.description}</p></div>
            </li>
          ))}
        </ol>
      </section>

      <footer className={styles.footer}>
        <span>Built for clear event operations.</span>
        <Link href="/demo">See how reservations work <Icon name="arrow-up-right" size={15} /></Link>
      </footer>
    </main>
  );
}

import { Suspense } from "react";
import Link from "next/link";
import { Icon, type IconName } from "./icons";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { Card } from "../ui/card";
import { createEventClock } from "@/lib/event-time";
import { getOverviewEventPreview, type EventPreviewState } from "@/lib/overview-data";
import styles from "./attendee-demo.module.css";

type NavigationItem = {
  label: string;
  icon: IconName;
  href: string;
  active?: boolean;
};

const navigation: NavigationItem[] = [
  { label: "Overview", icon: "grid", href: "#overview", active: true },
  { label: "Browse events", icon: "ticket", href: "/events" },
  { label: "How holds work", icon: "clock", href: "#flow-title" },
];

const reservationFlow = [
  {
    title: "Review event and ticket options",
    description: "See the published event details, local start time, and current availability.",
    detail: "Live catalog",
  },
  {
    title: "Hold your selected tickets",
    description: "Your quantity is held for 15 minutes. Retrying a request won’t create another hold.",
    detail: "15-minute hold",
  },
  {
    title: "Make held spots available again",
    description: "When the timer ends, held spots become available to other attendees.",
    detail: "Automatic",
  },
];

function Sidebar() {
  return (
    <aside aria-label="Attendee demo navigation" className={styles.sidebar}>
      <Link className={styles.brand} href="#overview">
        <span aria-hidden="true" className={styles.brandMark}>z</span>
        <span>zeno<span className={styles.brandPeriod}>.</span></span>
      </Link>

      <nav aria-label="Main" className={styles.sidebarNav}>
        <p className={styles.navLabel}>EXPLORE</p>
        <ul className={styles.navList}>
          {navigation.map((item) => (
            <li key={item.label}>
              <Link
                aria-current={item.active ? "page" : undefined}
                className={`${styles.navLink} ${item.active ? styles.navLinkActive : ""}`}
                href={item.href}
              >
                <Icon name={item.icon} size={16} />
                <span>
                  {item.label}
                </span>
              </Link>
            </li>
          ))}
        </ul>
      </nav>
    </aside>
  );
}

function Topbar() {
  return (
    <header className={styles.topbar}>
      <div aria-label="Breadcrumb" className={styles.breadcrumb}>
        <span>Zeno</span>
        <span aria-hidden="true" className={styles.breadcrumbSlash}>/</span>
        <strong>Attendee flow</strong>
      </div>
      <div className={styles.topbarActions}>
        <Badge className={styles.environmentBadge} variant="outline">DEMO</Badge>
        <Button href="/organizer" size="default" variant="outline">For organizers</Button>
      </div>
    </header>
  );
}

function ReservationFlowCard() {
  return (
    <Card as="section" aria-labelledby="flow-title" className={`${styles.panel} ${styles.flowPanel}`}>
      <div className={styles.panelHeading}>
        <h2 id="flow-title">How it works</h2>
      </div>
      <ol className={styles.flowList}>
        {reservationFlow.map((step, index) => (
          <li className={styles.flowStep} key={step.title}>
            <span aria-hidden="true" className={styles.stepNumber}>{String(index + 1).padStart(2, "0")}</span>
            <div className={styles.stepCopy}>
              <h3>{step.title}</h3>
              <p>{step.description}</p>
            </div>
            <span className={styles.stepDetail}>{step.detail}</span>
          </li>
        ))}
      </ol>

    </Card>
  );
}

function PublishedEventsCard({ preview }: { preview: EventPreviewState }) {
  return (
    <Card as="section" aria-labelledby="published-events-title" className={`${styles.panel} ${styles.eventsPanel}`}>
      <div className={styles.panelHeading}>
        <h2 id="published-events-title">Upcoming events</h2>
      </div>

      {preview.state === "unavailable" ? (
        <div className={styles.eventsEmpty} role="status">
          <p>We couldn’t load public events right now.</p>
          <p>Try again in a moment, or <Link href="/events">browse the event catalog</Link>.</p>
        </div>
      ) : preview.state === "empty" ? (
        <p className={styles.eventsEmpty}>No upcoming public events yet.</p>
      ) : (
        <ul className={styles.publishedEventList}>
          {preview.page.items.map((event) => {
            const clock = createEventClock(event.timezone);
            const date = clock.format(event.starts_at, {
              month: "short",
              day: "numeric",
              hour: "numeric",
              minute: "2-digit",
            });
            const timezone = clock.usesUtcFallback ? "UTC fallback" : clock.label;
            const venue = event.venue.kind === "physical" ? event.venue.city : "Online";
            return (
              <li key={event.id}>
                <Link aria-label={`Open ${event.title}`} className={styles.publishedEventRow} href={`/events/${event.id}`}>
                  <span className={styles.publishedEventDate}>{date} · {timezone}</span>
                  <span className={styles.publishedEventCopy}>
                    <span className={styles.publishedEventTitle}>{event.title}</span>
                    <span className={styles.publishedEventVenue}>{venue}</span>
                  </span>
                  <Icon name="arrow-up-right" size={14} />
                </Link>
              </li>
            );
          })}
        </ul>
      )}
    </Card>
  );
}

async function PublishedEventsPanel() {
  const preview = await getOverviewEventPreview();
  return <PublishedEventsCard preview={preview} />;
}

export function AttendeeDemo() {
  return (
    <div className={styles.appShell}>
      <a className={styles.skipLink} href="#overview">Skip to main content</a>
      <Sidebar />
      <div className={styles.mainColumn}>
        <Topbar />
        <main className={styles.mainContent} id="overview" tabIndex={-1}>
          <div className={styles.pageHeading}>
            <div>
              <h1>Try the attendee flow</h1>
              <p className={styles.pageSubhead}>Browse events and hold tickets for 15 minutes. No payment is collected, and confirming issues each guest their own ticket with a QR code.</p>
            </div>
            <Button className={styles.pageAction} href="/events">
              Browse events <Icon name="arrow-right" size={15} />
            </Button>
          </div>

          <div className={styles.contentGrid}>
            <Suspense fallback={<div className={styles.loadingPanels} role="status">Loading public event list…</div>}>
              <PublishedEventsPanel />
            </Suspense>
          </div>

          <div className={styles.flowSection}><ReservationFlowCard /></div>
        </main>
      </div>
    </div>
  );
}

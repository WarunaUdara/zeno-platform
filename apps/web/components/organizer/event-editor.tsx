"use client";

import { useRouter } from "next/navigation";
import { useRef, useState, useTransition } from "react";
import { createTicketType, publishEvent, updateDraftEvent, updateTicketType } from "@/app/organizer/actions";
import { Button } from "@/components/ui/button";
import { EventCoverForm } from "@/components/organizer/event-cover-form";
import { convertLocalDateTimesToTimeZone, isoToLocalDateTime } from "@/lib/local-datetime";
import {
  DEFAULT_TICKET_CURRENCY,
  EVENT_TIMEZONES,
  TICKET_CURRENCIES,
  matchCurrencyOption,
  matchTimezoneOption,
} from "@/lib/event-options";
import type { OrganizerEvent, OrganizerTicket } from "@/lib/organizer-service";
import { clearReservationRetry, loadReservationRetry, saveReservationRetry } from "@/lib/reservation-retry";
import { ticketCreateFingerprint } from "@/lib/ticket-create-retry";
import styles from "./event-editor.module.css";

function newIdempotencyKey() {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return btoa(String.fromCharCode(...bytes)).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/g, "");
}

function retryStorage() {
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
}

function EventDetailsForm({ organizationId, event }: { organizationId: string; event: OrganizerEvent }) {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [message, setMessage] = useState("");
  const [fieldError, setFieldError] = useState<string | undefined>();
  const [timeZone, setTimeZone] = useState(event.timezone);
  const [timeZoneInput, setTimeZoneInput] = useState(event.timezone);
  const [timeZoneMessage, setTimeZoneMessage] = useState("");
  const [startsAt, setStartsAt] = useState(isoToLocalDateTime(event.starts_at, event.timezone));
  const [endsAt, setEndsAt] = useState(isoToLocalDateTime(event.ends_at, event.timezone));
  const [venueKind, setVenueKind] = useState<string>(event.venue?.kind ?? "online");
  // An event stored in a zone this build does not list keeps its own value rather
  // than being moved to a default the organizer never chose.
  const knownTimezone = matchTimezoneOption(event.timezone);
  const attendeeDetails = useRef<HTMLDetailsElement>(null);

  function submit(submitEvent: React.FormEvent<HTMLFormElement>) {
    submitEvent.preventDefault();
    const form = submitEvent.currentTarget;
    const formData = new FormData(form);
    setMessage("");
    setFieldError(undefined);
    startTransition(async () => {
      const result = await updateDraftEvent(formData);
      if (result.ok) {
        setMessage("Event details saved.");
        router.refresh();
      } else {
        setFieldError(result.field);
        setMessage(result.message);
        if (result.field) {
          const invalidField = form.elements.namedItem(result.field);
          if (invalidField instanceof HTMLElement) {
            if (["accessibility_info", "cancellation_policy", "support_email"].includes(result.field)) {
              if (attendeeDetails.current) attendeeDetails.current.open = true;
            }
            invalidField.focus();
          }
        }
      }
    });
  }

  const physical = event.venue?.kind === "physical" ? event.venue : undefined;

  return (
    <form
      className={styles.form}
      onChangeCapture={() => {
        if (fieldError) {
          setFieldError(undefined);
          setMessage("");
        }
      }}
      onSubmit={submit}
    >
      <input name="organization_id" type="hidden" value={organizationId} />
      <input name="event_id" type="hidden" value={event.id} />
      <div className={styles.fields}>
        <label className={styles.field} htmlFor="event-title"><span>Event name <b aria-hidden="true">*</b></span><input autoComplete="off" defaultValue={event.title} id="event-title" maxLength={160} name="title" required /></label>
        <label className={styles.field} htmlFor="event-summary"><span>Short description <small>Optional</small></span><textarea defaultValue={event.summary ?? ""} id="event-summary" maxLength={1000} name="summary" rows={3} /></label>
        <label className={styles.field} htmlFor="event-timezone"><span>Event timezone</span><select id="event-timezone" name="timezone" onBlur={() => {
          const nextTimeZone = timeZoneInput.trim();
          if (nextTimeZone === timeZone) return;
          const converted = convertLocalDateTimesToTimeZone({ startsAt, endsAt }, timeZone, nextTimeZone);
          if (!converted) {
            setTimeZoneInput(timeZone);
            setTimeZoneMessage("Enter a valid timezone and check that both event times exist in the current timezone before changing it.");
            return;
          }
          setTimeZone(nextTimeZone);
          setTimeZoneInput(nextTimeZone);
          setStartsAt(converted.startsAt);
          setEndsAt(converted.endsAt);
          setTimeZoneMessage("Times were adjusted to keep the same moments for guests.");
        }} onChange={(change) => setTimeZoneInput(change.target.value)} required value={timeZoneInput}>
          {knownTimezone ? null : <option value={event.timezone}>{event.timezone}</option>}
          {EVENT_TIMEZONES.map((option) => (
            <option key={option.value} value={option.value}>{option.label}</option>
          ))}
        </select><small>Times are shown to guests in this timezone. Changing it keeps the same event moments.</small>{timeZoneMessage ? <small aria-live="polite" role="status">{timeZoneMessage}</small> : null}</label>
        <div className={styles.twoFields}>
          <label className={styles.field} htmlFor="event-start"><span>Starts <b aria-hidden="true">*</b></span><input onChange={(change) => setStartsAt(change.target.value)} value={startsAt} id="event-start" name="starts_at" required type="datetime-local" /></label>
          <label className={styles.field} htmlFor="event-end"><span>Ends <small>Optional</small></span><input onChange={(change) => setEndsAt(change.target.value)} value={endsAt} id="event-end" name="ends_at" type="datetime-local" /></label>
        </div>
        <label className={styles.field} htmlFor="venue-kind"><span>Where is it?</span><select id="venue-kind" name="venue_kind" onChange={(change) => setVenueKind(change.target.value)} value={venueKind}><option value="online">Online</option><option value="physical">In person</option></select></label>
        {venueKind === "physical" ? (
          <div className={styles.twoFields}>
            <label className={styles.field} htmlFor="venue-name"><span>Venue name <b aria-hidden="true">*</b></span><input autoComplete="organization" defaultValue={physical?.name ?? ""} id="venue-name" maxLength={160} name="venue_name" required /></label>
            <label className={styles.field} htmlFor="venue-city"><span>City <b aria-hidden="true">*</b></span><input autoComplete="address-level2" defaultValue={physical?.city ?? ""} id="venue-city" maxLength={120} name="city" required /></label>
            <label className={styles.field} htmlFor="country-code"><span>Country code <b aria-hidden="true">*</b></span><input autoComplete="country" defaultValue={physical?.country_code ?? ""} id="country-code" maxLength={2} name="country_code" placeholder="LK" required /></label>
          </div>
        ) : null}
      </div>
      <details className={styles.optionalDetails} ref={attendeeDetails}>
        <summary>
          <span>For attendees <small>Optional</small></span>
          <small>Access, cancellation, and contact details</small>
        </summary>
        <p className={styles.hint}>These details appear on the public event page. Leave any item blank if it does not apply.</p>
        <div className={styles.fields}>
          <label className={styles.field} htmlFor="event-accessibility">
            <span>Accessibility and access</span>
            <textarea aria-describedby={fieldError === "accessibility_info" ? "event-details-error" : undefined} aria-invalid={fieldError === "accessibility_info" || undefined} defaultValue={event.accessibility_info ?? ""} id="event-accessibility" maxLength={2000} name="accessibility_info" placeholder="For example: step-free entrance, accessible restroom, or who to contact about access needs." rows={3} />
          </label>
          <label className={styles.field} htmlFor="event-cancellation">
            <span>Cancellation and transfer terms</span>
            <textarea aria-describedby={fieldError === "cancellation_policy" ? "event-details-error" : undefined} aria-invalid={fieldError === "cancellation_policy" || undefined} defaultValue={event.cancellation_policy ?? ""} id="event-cancellation" maxLength={2000} name="cancellation_policy" placeholder="Share the terms guests should know before reserving." rows={3} />
          </label>
          <label className={styles.field} htmlFor="event-support-email">
            <span>Questions from guests</span>
            <input aria-describedby={fieldError === "support_email" ? "event-details-error" : undefined} aria-invalid={fieldError === "support_email" || undefined} autoComplete="email" defaultValue={event.support_email ?? ""} id="event-support-email" maxLength={254} name="support_email" placeholder="events@example.com" type="email" />
            <small>This email address will be public on the event page.</small>
          </label>
        </div>
      </details>
      <div className={styles.inlineFooter}>
        {message ? <p aria-live="polite" className={message === "Event details saved." ? styles.success : styles.formError} id="event-details-error" role="status">{message}</p> : null}
        <Button disabled={pending} type="submit" variant="outline">{pending ? "Saving…" : "Save event details"}</Button>
      </div>
    </form>
  );
}

function TicketFields({ organizationId, ticket, event, now }: { organizationId: string; ticket?: OrganizerTicket; event: OrganizerEvent; now: string }) {
  const zone = event.timezone;
  const knownCurrency = ticket ? matchCurrencyOption(ticket.currency) : undefined;
  return (
    <>
      <input name="organization_id" type="hidden" value={organizationId} />
      <input name="event_id" type="hidden" value={event.id} />
      <input name="timezone" type="hidden" value={zone} />
      {ticket ? <input name="ticket_type_id" type="hidden" value={ticket.id} /> : null}
      <div className={styles.fields}>
        <label className={styles.field} htmlFor={`${ticket?.id ?? "new"}-ticket-name`}><span>Ticket name</span><input defaultValue={ticket?.name ?? "General admission"} id={`${ticket?.id ?? "new"}-ticket-name`} maxLength={120} name="name" required /></label>
        <label className={styles.field} htmlFor={`${ticket?.id ?? "new"}-ticket-description`}><span>What&apos;s included <small>Optional</small></span><input defaultValue={ticket?.description ?? ""} id={`${ticket?.id ?? "new"}-ticket-description`} maxLength={500} name="description" /></label>
        <div className={styles.twoFields}>
          <label className={styles.field} htmlFor={`${ticket?.id ?? "new"}-ticket-amount`}><span>Price shown to guests</span><input defaultValue={ticket?.amount ?? "0.00"} id={`${ticket?.id ?? "new"}-ticket-amount`} inputMode="decimal" maxLength={32} name="amount" placeholder="0.00" required /></label>
          <label className={styles.field} htmlFor={`${ticket?.id ?? "new"}-ticket-currency`}><span>Currency</span><select defaultValue={ticket?.currency ?? DEFAULT_TICKET_CURRENCY} id={`${ticket?.id ?? "new"}-ticket-currency`} name="currency" required>
          {knownCurrency ? null : ticket ? <option value={ticket.currency}>{ticket.currency}</option> : null}
          {TICKET_CURRENCIES.map((option) => (
            <option key={option.value} value={option.value}>{option.label}</option>
          ))}
        </select></label>
        </div>
        <label className={styles.field} htmlFor={`${ticket?.id ?? "new"}-ticket-capacity`}><span>Places available</span><input defaultValue={ticket?.capacity ?? 100} id={`${ticket?.id ?? "new"}-ticket-capacity`} max={2147483647} min={1} name="capacity" required type="number" /><small>Capacity cannot be reduced below places already sold or held.</small></label>
        <div className={styles.twoFields}>
          <label className={styles.field} htmlFor={`${ticket?.id ?? "new"}-sale-start`}><span>Sales open</span><input defaultValue={isoToLocalDateTime(ticket?.sale_starts_at ?? now, zone)} id={`${ticket?.id ?? "new"}-sale-start`} name="sale_starts_at" required type="datetime-local" /></label>
          <label className={styles.field} htmlFor={`${ticket?.id ?? "new"}-sale-end`}><span>Sales close</span><input defaultValue={isoToLocalDateTime(ticket?.sale_ends_at ?? event.starts_at, zone)} id={`${ticket?.id ?? "new"}-sale-end`} name="sale_ends_at" required type="datetime-local" /></label>
        </div>
      </div>
      <p className={styles.notice}>Ticket prices and capacities are visible to attendees upon publication.</p>
    </>
  );
}

function NewTicketForm({ organizationId, event, now }: { organizationId: string; event: OrganizerEvent; now: string }) {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [message, setMessage] = useState("");
  const keyInput = useRef<HTMLInputElement>(null);
  const retryRef = useRef<{ fingerprint: string; key: string } | null>(null);

  function submit(submitEvent: React.FormEvent<HTMLFormElement>) {
    submitEvent.preventDefault();
    const form = submitEvent.currentTarget;
    const formData = new FormData(form);
    setMessage("");

    startTransition(async () => {
      let fingerprint: string;
      try {
        fingerprint = await ticketCreateFingerprint(event.id, organizationId, formData);
      } catch {
        setMessage("Could not prepare a safe retry. Your details are still here; try again.");
        return;
      }
      const storage = retryStorage();
      const saved = loadReservationRetry(storage, fingerprint);
      const key = retryRef.current?.fingerprint === fingerprint
        ? retryRef.current.key
        : saved?.key ?? newIdempotencyKey();
      retryRef.current = { fingerprint, key };
      saveReservationRetry(storage, { fingerprint, key });
      if (keyInput.current) keyInput.current.value = key;
      formData.set("idempotency_key", key);
      const result = await createTicketType(formData);
      if (result.ok) {
        clearReservationRetry(storage, key);
        if (retryRef.current?.key === key) retryRef.current = null;
        setMessage("Ticket option added.");
        router.refresh();
      } else {
        setMessage(result.message);
      }
    });
  }

  return (
    <form className={styles.form} onSubmit={submit}>
      <input name="idempotency_key" ref={keyInput} type="hidden" />
      <TicketFields event={event} now={now} organizationId={organizationId} />
      {message ? <p aria-live="polite" className={message === "Ticket option added." ? styles.success : styles.formError} role="status">{message}</p> : null}
      <Button disabled={pending} type="submit" variant="outline">{pending ? "Adding ticket…" : "Add ticket option"}</Button>
    </form>
  );
}

function ExistingTicket({ organizationId, ticket, event, now }: { organizationId: string; ticket: OrganizerTicket; event: OrganizerEvent; now: string }) {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [message, setMessage] = useState("");

  function submit(submitEvent: React.FormEvent<HTMLFormElement>) {
    submitEvent.preventDefault();
    const formData = new FormData(submitEvent.currentTarget);
    setMessage("");
    startTransition(async () => {
      const result = await updateTicketType(formData);
      if (result.ok) {
        setMessage("Ticket option saved.");
        router.refresh();
      } else {
        setMessage(result.message);
      }
    });
  }

  return (
    <details className={styles.ticketDetails}>
      <summary>
        <span className={styles.ticketName}>{ticket.name}</span>
        <span className={styles.ticketMeta}>{ticket.amount} {ticket.currency} · {ticket.capacity} places</span>
      </summary>
      <form className={styles.form} onSubmit={submit}>
        <TicketFields event={event} now={now} organizationId={organizationId} ticket={ticket} />
        {message ? <p aria-live="polite" className={message === "Ticket option saved." ? styles.success : styles.formError} role="status">{message}</p> : null}
        <Button disabled={pending} type="submit" variant="outline">{pending ? "Saving…" : "Save ticket option"}</Button>
      </form>
    </details>
  );
}

function PublishControl({ organizationId, event, hasTickets }: { organizationId: string; event: OrganizerEvent; hasTickets: boolean }) {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [message, setMessage] = useState("");
  const [published, setPublished] = useState(false);

  function submit(submitEvent: React.FormEvent<HTMLFormElement>) {
    submitEvent.preventDefault();
    const formData = new FormData(submitEvent.currentTarget);
    setMessage("");
    startTransition(async () => {
      const result = await publishEvent(formData);
      if (result.ok) {
        setPublished(true);
        router.refresh();
      } else {
        setMessage(result.message);
      }
    });
  }

  if (event.visibility === "public" || published) {
    return (
      <section aria-labelledby="published-title" className={styles.publishSection}>
        <p className={styles.eyebrow}>PUBLISHED</p>
        <h2 id="published-title">Your event is live.</h2>
        <p className={styles.notice}>Ticket options are published and available for attendee reservations.</p>
        <Button href={`/events/${encodeURIComponent(event.id)}`} variant="outline">View public event page</Button>
      </section>
    );
  }

  return (
    <section aria-labelledby="publish-title" className={styles.publishSection}>
      <p className={styles.eyebrow}>READY FOR GUESTS?</p>
      <h2 id="publish-title">Publish this event</h2>
      <p className={styles.notice}>Publishing makes the event and ticket options live to attendees. Please verify dates, times, and venue before proceeding.</p>
      {!hasTickets ? <p className={styles.hint}>Add at least one ticket option before publishing.</p> : null}
      {message ? <p aria-live="polite" className={styles.formError} role="alert">{message}</p> : null}
      <form onSubmit={submit}>
        <input name="organization_id" type="hidden" value={organizationId} />
        <input name="event_id" type="hidden" value={event.id} />
        <label className={styles.publishAcknowledgement}>
          <input name="publish_acknowledged" required type="checkbox" />
          <span>I’ve verified the event details and want to publish.</span>
        </label>
        <Button disabled={pending || !hasTickets} size="lg" type="submit">{pending ? "Publishing…" : "Publish event"}</Button>
      </form>
    </section>
  );
}

export function OrganizerEventEditor({
  organizationId,
  event,
  tickets,
  now,
  canEdit,
}: {
  organizationId: string;
  event: OrganizerEvent;
  tickets: OrganizerTicket[];
  now: string;
  canEdit: boolean;
}) {
  const published = event.visibility === "public";
  const readOnly = published || !canEdit;

  return (
    <div className={styles.editorStack}>
      <section aria-labelledby="details-heading" className={styles.editorSection}>
        <div className={styles.sectionHeading}>
          <span className={styles.stepNumber}>1</span>
          <div><h2 id="details-heading">Event details</h2><p>{published ? "Public and read-only after publishing in this demo." : canEdit ? "This stays private until you publish." : "Draft details are visible to analysts. Owners, admins, and event managers can make changes."}</p></div>
          <span className={`${styles.status} ${published ? styles.statusLive : ""}`}>{published ? "Published" : "Draft"}</span>
        </div>
        {readOnly ? (
          <dl className={styles.readOnlyDetails}>
            <div><dt>Name</dt><dd>{event.title}</dd></div>
            {event.summary ? <div><dt>About</dt><dd>{event.summary}</dd></div> : null}
            <div><dt>Start</dt><dd>{event.starts_at ? `${isoToLocalDateTime(event.starts_at, event.timezone).replace("T", " ")} · ${event.timezone}` : "Not set"}</dd></div>
            {event.ends_at ? <div><dt>End</dt><dd>{`${isoToLocalDateTime(event.ends_at, event.timezone).replace("T", " ")} · ${event.timezone}`}</dd></div> : null}
            <div><dt>Venue</dt><dd>{event.venue?.kind === "physical" ? `${event.venue.name}, ${event.venue.city}` : "Online"}</dd></div>
          </dl>
        ) : <EventDetailsForm event={event} organizationId={organizationId} />}
      </section>

      <EventCoverForm canEdit={!readOnly} event={event} organizationId={organizationId} />

      <section aria-labelledby="tickets-heading" className={styles.editorSection}>
        <div className={styles.sectionHeading}>
          <span className={styles.stepNumber}>2</span>
          <div><h2 id="tickets-heading">Ticket options</h2><p>{canEdit ? "Set the available quantity and when sales close." : "Review the configured price and capacity."}</p></div>
        </div>
        {tickets.length ? (
          <ul className={styles.ticketList}>
            {tickets.map((ticket) => (
              <li key={ticket.id}>
                {readOnly ? (
                  <div className={styles.ticketSummary}><span className={styles.ticketName}>{ticket.name}</span>{ticket.description ? <span>{ticket.description}</span> : null}<span className={styles.ticketMeta}>{ticket.amount} {ticket.currency} · {ticket.capacity} places</span></div>
                ) : <ExistingTicket event={event} now={now} organizationId={organizationId} ticket={ticket} />}
              </li>
            ))}
          </ul>
        ) : (
          <p className={styles.emptyTickets}>{canEdit ? "No ticket options yet. Add at least one before publishing." : "No ticket options are configured yet."}</p>
        )}
        {!readOnly ? (
          <details className={styles.addTicket}>
            <summary>{tickets.length ? "Add another ticket option" : "Add your first ticket option"}</summary>
            <NewTicketForm event={event} now={now} organizationId={organizationId} />
          </details>
        ) : null}
      </section>

      {!readOnly ? <PublishControl event={event} hasTickets={tickets.length > 0} organizationId={organizationId} /> : null}
    </div>
  );
}

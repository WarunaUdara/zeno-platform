"use client";

import { useRouter } from "next/navigation";
import { useEffect, useRef, useState, useTransition } from "react";
import { createDraftEvent } from "@/app/organizer/actions";
import { Button } from "@/components/ui/button";
import {
  clearEventCreateRetry,
  eventCreateFingerprint,
  loadEventCreateRetry,
  saveEventCreateRetry,
} from "@/lib/event-create-retry";
import styles from "./event-editor.module.css";

function slugFromTitle(title: string) {
  return title
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 62)
    .replace(/-$/g, "");
}

function newSlug(title: string) {
  const stem = slugFromTitle(title) || "event";
  const suffix = crypto.randomUUID().replaceAll("-", "").slice(0, 8).toLowerCase();
  return `${stem}-${suffix}`.slice(0, 80).replace(/-$/g, "");
}

function retryStorage() {
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
}

export function EventDraftForm({ organizationId, organizationCursor }: { organizationId: string; organizationCursor: string | null }) {
  const router = useRouter();
  const [pending, startTransition] = useTransition();
  const [message, setMessage] = useState("");
  const [venueKind, setVenueKind] = useState("online");
  const [title, setTitle] = useState("");
  const [fieldError, setFieldError] = useState<string | undefined>();
  const requestSlug = useRef<HTMLInputElement>(null);
  const retryRef = useRef<{ fingerprint: string; slug: string } | null>(null);
  const timeZoneInput = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (zone && timeZoneInput.current) timeZoneInput.current.value = zone;
  }, []);

  function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    const formData = new FormData(form);
    setMessage("");
    setFieldError(undefined);

    startTransition(async () => {
      let fingerprint: string;
      try {
        fingerprint = await eventCreateFingerprint(organizationId, formData);
      } catch {
        setMessage("Could not prepare a safe retry. Your details are still here; try again.");
        return;
      }
      const storage = retryStorage();
      const saved = loadEventCreateRetry(storage, organizationId, fingerprint);
      const retry = retryRef.current?.fingerprint === fingerprint
        ? retryRef.current
        : saved ?? { fingerprint, slug: newSlug(title) };
      retryRef.current = retry;
      saveEventCreateRetry(storage, organizationId, retry);
      if (requestSlug.current) requestSlug.current.value = retry.slug;
      formData.set("slug", retry.slug);

      const result = await createDraftEvent(formData);
      if (result.ok) {
        if (result.eventId) {
          clearEventCreateRetry(storage, organizationId);
          const organizationQuery = new URLSearchParams({ organization: organizationId });
          if (organizationCursor) organizationQuery.set("org_cursor", organizationCursor);
          router.replace(`/organizer/events/${encodeURIComponent(result.eventId)}?${organizationQuery.toString()}`);
          router.refresh();
        }
        return;
      }
      setFieldError(result.field);
      setMessage(result.message);
      if (result.field) {
        const invalidField = form.elements.namedItem(result.field);
        if (invalidField instanceof HTMLElement) invalidField.focus();
      }
    });
  }

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
      <input name="slug" ref={requestSlug} type="hidden" />
      <section aria-labelledby="event-basics-heading" className={styles.formSection}>
        <div className={styles.sectionHeading}>
          <span className={styles.stepNumber}>1</span>
          <div><h2 id="event-basics-heading">Event basics</h2><p>Guests will see this information on the event page.</p></div>
        </div>
        <div className={styles.fields}>
          <label className={styles.field} htmlFor="event-title">
            <span>Event name <b aria-hidden="true">*</b></span>
            <input aria-describedby={fieldError === "title" ? "event-form-error" : undefined} aria-invalid={fieldError === "title" || undefined} autoFocus autoComplete="off" id="event-title" maxLength={160} name="title" onChange={(event) => setTitle(event.target.value)} placeholder="e.g. Community design night" required value={title} />
          </label>
          <label className={styles.field} htmlFor="event-summary">
            <span>Short description <small>Optional</small></span>
            <textarea id="event-summary" maxLength={1000} name="summary" placeholder="What will guests take part in?" rows={3} />
          </label>
          <label className={styles.field} htmlFor="event-timezone">
            <span>Event timezone</span>
            <input aria-describedby={fieldError === "starts_at" ? "event-form-error" : undefined} aria-invalid={fieldError === "starts_at" || undefined} autoCapitalize="none" autoComplete="off" defaultValue="Asia/Colombo" id="event-timezone" name="timezone" ref={timeZoneInput} required />
            <small>Times are shown to guests in this timezone.</small>
          </label>
          <div className={styles.twoFields}>
            <label className={styles.field} htmlFor="event-start">
              <span>Starts <b aria-hidden="true">*</b></span>
              <input aria-describedby={fieldError === "starts_at" ? "event-form-error" : undefined} aria-invalid={fieldError === "starts_at" || undefined} id="event-start" name="starts_at" required type="datetime-local" />
            </label>
            <label className={styles.field} htmlFor="event-end">
              <span>Ends <small>Optional</small></span>
              <input aria-describedby={fieldError === "ends_at" ? "event-form-error" : undefined} aria-invalid={fieldError === "ends_at" || undefined} id="event-end" name="ends_at" type="datetime-local" />
            </label>
          </div>
          <label className={styles.field} htmlFor="venue-kind">
            <span>Where is it?</span>
            <select aria-describedby={fieldError === "venue_kind" ? "event-form-error" : undefined} aria-invalid={fieldError === "venue_kind" || undefined} id="venue-kind" name="venue_kind" onChange={(event) => setVenueKind(event.target.value)} value={venueKind}>
              <option value="online">Online</option>
              <option value="physical">In person</option>
            </select>
          </label>
          {venueKind === "physical" ? (
            <div className={styles.twoFields}>
              <label className={styles.field} htmlFor="venue-name"><span>Venue name <b aria-hidden="true">*</b></span><input aria-describedby={fieldError === "venue_name" ? "event-form-error" : undefined} aria-invalid={fieldError === "venue_name" || undefined} autoComplete="organization" id="venue-name" maxLength={160} name="venue_name" required /></label>
              <label className={styles.field} htmlFor="venue-city"><span>City <b aria-hidden="true">*</b></span><input autoComplete="address-level2" id="venue-city" maxLength={120} name="city" required /></label>
              <label className={styles.field} htmlFor="country-code"><span>Country code <b aria-hidden="true">*</b></span><input autoComplete="country" id="country-code" maxLength={2} name="country_code" placeholder="LK" required /></label>
            </div>
          ) : null}
        </div>
      </section>

      <div className={styles.formFooter}>
        {message ? <p aria-live="polite" className={styles.formError} id="event-form-error" role="alert">{message}</p> : null}
        <Button disabled={pending || title.trim().length === 0} size="lg" type="submit">
          {pending ? "Saving your draft…" : "Save and continue"}
        </Button>
        <p>Your event stays private as a draft until you publish it.</p>
      </div>
    </form>
  );
}

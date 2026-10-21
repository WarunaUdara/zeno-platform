"use client";

import Image from "next/image";
import { useRouter } from "next/navigation";
import { useRef, useState, useTransition } from "react";
import { removeEventCoverImage, saveEventCoverImage } from "@/app/organizer/actions";
import { Button } from "@/components/ui/button";
import type { OrganizerEvent } from "@/lib/organizer-service";
import styles from "./event-editor.module.css";

export function EventCoverForm({
  event,
  organizationId,
  canEdit,
}: {
  event: OrganizerEvent;
  organizationId: string;
  canEdit: boolean;
}) {
  const router = useRouter();
  const formRef = useRef<HTMLFormElement>(null);
  const [pending, startTransition] = useTransition();
  const [message, setMessage] = useState("");
  const [failed, setFailed] = useState(false);
  const hasImage = Boolean(event.cover_image_key);

  function submit(submitEvent: React.FormEvent<HTMLFormElement>) {
    submitEvent.preventDefault();
    const form = submitEvent.currentTarget;
    const formData = new FormData(form);
    setMessage("");
    setFailed(false);
    startTransition(async () => {
      const result = await saveEventCoverImage(organizationId, event.id, formData);
      setMessage(result.ok ? "Event image saved." : result.message);
      setFailed(!result.ok);
      if (result.ok) {
        formRef.current?.reset();
        router.refresh();
      }
    });
  }

  function remove() {
    setMessage("");
    setFailed(false);
    startTransition(async () => {
      const result = await removeEventCoverImage(organizationId, event.id);
      setMessage(result.ok ? "Event image removed." : result.message);
      setFailed(!result.ok);
      if (result.ok) router.refresh();
    });
  }

  return (
    <section aria-labelledby="cover-image-heading" className={styles.editorSection}>
      <div className={styles.sectionHeading}>
        <span className={styles.stepNumber} aria-hidden="true">↗</span>
        <div>
          <h2 id="cover-image-heading">Event image</h2>
          <p>{canEdit ? "Add one image to help guests recognize this event." : hasImage ? "This image appears on the public event page." : "No event image has been added."}</p>
        </div>
      </div>
      {hasImage ? (
        <div className={styles.coverPreview}>
          <Image
            alt={event.cover_image_alt ?? "Event cover image"}
            className={styles.coverPreviewImage}
            fill
            sizes="(max-width: 760px) 100vw, 720px"
            src={event.visibility === "public"
              ? `/api/events/${encodeURIComponent(event.id)}/cover`
              : `/api/organizer/organizations/${encodeURIComponent(organizationId)}/events/${encodeURIComponent(event.id)}/cover`}
            unoptimized
          />
          {canEdit ? (
            <Button disabled={pending} onClick={remove} type="button" variant="outline">
              {pending ? "Removing…" : "Remove image"}
            </Button>
          ) : null}
        </div>
      ) : null}
      {canEdit ? (
        <form className={styles.form} onSubmit={submit} ref={formRef}>
          <label className={styles.field} htmlFor="event-cover-file">
            <span>{hasImage ? "Replace image" : "Choose an image"}</span>
            <input accept="image/jpeg,image/png,image/webp" id="event-cover-file" name="image" required type="file" />
            <small>JPEG, PNG, or WebP · up to 5 MB</small>
          </label>
          <label className={styles.field} htmlFor="event-cover-alt">
            <span>Describe the image</span>
            <input
              defaultValue={event.cover_image_alt ?? ""}
              id="event-cover-alt"
              maxLength={240}
              name="alt"
              placeholder="A speaker on stage at the annual design summit"
              required
            />
            <small>Guests using screen readers hear this description.</small>
          </label>
          {message ? <p aria-live="polite" className={failed ? styles.formError : styles.success} role={failed ? "alert" : "status"}>{message}</p> : null}
          <Button disabled={pending} type="submit" variant="outline">{pending ? "Uploading image…" : hasImage ? "Save new image" : "Upload image"}</Button>
        </form>
      ) : message ? <p aria-live="polite" className={failed ? styles.formError : styles.success} role={failed ? "alert" : "status"}>{message}</p> : null}
    </section>
  );
}

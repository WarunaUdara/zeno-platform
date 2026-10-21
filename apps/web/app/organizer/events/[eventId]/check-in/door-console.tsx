"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import styles from "./check-in.module.css";

type Outcome =
  | { outcome: "admitted"; ticket_id: string; sequence: number; reference: string; attendee_name?: string | null }
  | { outcome: "already_admitted"; ticket_id: string; sequence: number; reference: string; checked_in_at: string }
  | { outcome: "voided"; ticket_id: string; sequence: number; reference: string }
  | { outcome: "unknown" };

type ScanResult = { outcome: Outcome; replayed: boolean };

/**
 * The door screen.
 *
 * Designed for a volunteer on their first shift, on a cracked phone, in bad
 * light, with a queue behind them. Three rules follow from that:
 *
 * - One question, one answer. Admitted or not. Nothing else competes for attention.
 * - The answer is words, not colour alone. A green screen is invisible to someone
 *   who cannot distinguish it from red, and this screen is read in a hurry.
 * - The previous result stays on screen. Staff glance up from the queue and need
 *   to see what just happened, not an empty box waiting for the next scan.
 */
export function DoorConsole({
  organizationId,
  eventId,
  initialAttendance,
}: {
  organizationId: string;
  eventId: string;
  initialAttendance: { issued: number; admitted: number; awaiting: number; voided: number };
}) {
  const [code, setCode] = useState("");
  const [results, setResults] = useState<ScanResult[]>([]);
  const [scanning, setScanning] = useState(false);
  const [failure, setFailure] = useState("");
  const [attendance, setAttendance] = useState(initialAttendance);
  const inputRef = useRef<HTMLInputElement>(null);

  const refreshAttendance = useCallback(async () => {
    try {
      const response = await fetch(
        `/api/organizer/organizations/${encodeURIComponent(organizationId)}/events/${encodeURIComponent(eventId)}/attendance`,
      );
      if (response.ok) setAttendance(await response.json());
    } catch {
      // A stale counter is better than an error replacing the scan result.
    }
  }, [organizationId, eventId]);

  const scan = useCallback(
    async (raw: string) => {
      const trimmed = raw.trim();
      if (!trimmed) return;
      setScanning(true);
      setFailure("");
      try {
        // A fresh action id per scan keeps a retry from being mistaken for a new
        // scan. It is generated here rather than derived from time so two taps
        // in the same millisecond cannot collide.
        const response = await fetch(
          `/api/organizer/organizations/${encodeURIComponent(organizationId)}/events/${encodeURIComponent(eventId)}/check-ins`,
          {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
              codes: [trimmed],
              scanner_action_id: newActionId(),
            }),
          },
        );
        const body = await response.json().catch(() => null);
        if (!response.ok) {
          setFailure(
            response.status === 404
              ? "This door is not set up. Check you are signed in to the right workspace."
              : "Could not reach the door service. Try the next guest.",
          );
          setResults([]);
          return;
        }
        setResults((body ?? []) as ScanResult[]);
        setCode("");
        await refreshAttendance();
      } catch {
        setFailure("Could not reach the door service. Try the next guest.");
      } finally {
        setScanning(false);
        inputRef.current?.focus();
      }
    },
    [organizationId, eventId, refreshAttendance],
  );

  // Returning to the top of the page after the next guest is a common staff
  // habit; re-focusing the field keeps the screen ready without being asked.
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const headline = results.length === 1 ? describe(results[0].outcome) : null;

  return (
    <section className={styles.console}>
      <header className={styles.header}>
        <div>
          <p className={styles.eyebrow}>AT THE DOOR</p>
          <h1>Check guests in</h1>
        </div>
        <dl className={styles.counts}>
          <div>
            <dt>Inside</dt>
            <dd>{attendance.admitted}</dd>
          </div>
          <div>
            <dt>Still coming</dt>
            <dd>{attendance.awaiting}</dd>
          </div>
          <div>
            <dt>Tickets</dt>
            <dd>{attendance.issued}</dd>
          </div>
        </dl>
      </header>

      <form
        className={styles.scanForm}
        onSubmit={(event) => {
          event.preventDefault();
          void scan(code);
        }}
      >
        <label htmlFor="door-code">Scan a code or type a reference</label>
        <div className={styles.scanRow}>
          <input
            autoCapitalize="characters"
            autoComplete="off"
            autoFocus
            id="door-code"
            inputMode="text"
            onChange={(event) => setCode(event.target.value)}
            placeholder="Scan, or type the code"
            ref={inputRef}
            value={code}
          />
          <Button disabled={scanning || code.trim().length === 0} size="lg" type="submit">
            {scanning ? "Checking…" : "Check in"}
          </Button>
        </div>
      </form>

      {failure ? (
        <p className={styles.failure} role="alert">
          {failure}
        </p>
      ) : null}

      {headline ? (
        <div
          aria-live="assertive"
          className={`${styles.result} ${
            headline.tone === "yes"
              ? styles.resultYes
              : headline.tone === "care"
                ? styles.resultCare
                : styles.resultNo
          }`}
          role="status"
        >
          <strong className={styles.resultVerdict}>{headline.verdict}</strong>
          <span className={styles.resultDetail}>{headline.detail}</span>
          {results[0].outcome.outcome === "already_admitted" ? (
            <span className={styles.resultMeta}>
              Admitted at {new Date(results[0].outcome.checked_in_at).toLocaleTimeString()}
            </span>
          ) : null}
          {results[0].outcome.outcome === "admitted" ? (
            <span className={styles.resultMeta}>
              Ticket {results[0].outcome.sequence}
              {results[0].outcome.attendee_name
                ? ` · ${results[0].outcome.attendee_name}`
                : ` · ${results[0].outcome.reference}`}
            </span>
          ) : null}
        </div>
      ) : (
        <p className={styles.idle} role="status">
          Waiting for a code.
        </p>
      )}
    </section>
  );
}

/**
 * Say what the staff member should do next, not what the database recorded.
 *
 * "Already admitted" is deliberately not a failure: a colleague scanning a second
 * ticket for the same booking needs to hear that the guest is fine, not that
 * something is wrong.
 */
function describe(outcome: Outcome): { verdict: string; detail: string; tone: "yes" | "care" | "no" } {
  switch (outcome.outcome) {
    case "admitted":
      return { verdict: "In", detail: "Let them in.", tone: "yes" };
    case "already_admitted":
      return {
        verdict: "Already in",
        detail: "This ticket was already used. Look for another ticket.",
        tone: "care",
      };
    case "voided":
      return {
        verdict: "Not valid",
        detail: "This ticket was cancelled. Send them to the organizer.",
        tone: "no",
      };
    default:
      return {
        verdict: "Not recognised",
        detail: "Check the code, or send them to the organizer.",
        tone: "no",
      };
  }
}

function newActionId(): string {
  const bytes = new Uint8Array(9);
  crypto.getRandomValues(bytes);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}
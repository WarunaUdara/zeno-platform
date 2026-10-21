"use client";

import { useEffect, useMemo, useRef, useState, useSyncExternalStore, useTransition, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import type { components } from "@zeno/api-client";
import { createEventClock } from "@/lib/event-time";
import { MAX_RESERVATION_QUANTITY, parseReservationQuantity } from "@/lib/reservation-quantity";
import {
  readReservationReleaseStatus,
  readReservationResponseBody,
} from "@/lib/reservation-response";
import { supportReference as getSupportReference } from "@/lib/support-reference";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  clearReservationConfirmation,
  clearReservationRetry,
  getReservationConfirmationSnapshot,
  loadReservationRetry,
  parseReservationConfirmation,
  saveReservationConfirmation,
  saveReservationRetry,
  subscribeToReservationConfirmation,
} from "@/lib/reservation-retry";
import styles from "./event-page.module.css";
import { BookingDetails, TicketWallet, type ConfirmedOrder } from "./ticket-checkout-details";

type TicketType = components["schemas"]["PublishedTicketType"];
type Reservation = components["schemas"]["Reservation"];
const MONEY_FORMATTER = new Intl.NumberFormat("en");

function formatMoney(amount: string, currency: string) {
  const [whole, fraction] = amount.split(".");
  const groupedWhole = MONEY_FORMATTER.format(BigInt(whole));
  return currency + " " + groupedWhole + (fraction ? "." + fraction : "");
}

function multiplyAmount(amount: string, quantity: number) {
  const [whole, fraction = ""] = amount.split(".");
  const scale = BigInt(10) ** BigInt(fraction.length);
  const amountInMinorUnits = BigInt(whole) * scale + BigInt(fraction || "0");
  const totalInMinorUnits = amountInMinorUnits * BigInt(quantity);
  const totalWhole = (totalInMinorUnits / scale).toString();
  if (!fraction) return totalWhole;

  const totalFraction = (totalInMinorUnits % scale).toString().padStart(fraction.length, "0");
  return totalWhole + "." + totalFraction;
}

function newIdempotencyKey() {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

function reservationRetryStorage(): Storage | null {
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
}

function formatRemainingTime(seconds: number) {
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  return `${minutes}:${remainingSeconds.toString().padStart(2, "0")}`;
}

function ReservationConfirmation({
  reservation,
  eventTimezone,
  onCheckAvailability,
  onHoldReleased,
  bookingStep,
  confirmed,
}: {
  reservation: Reservation & { idempotency_key?: string };
  eventTimezone: string;
  onCheckAvailability: () => void;
  onHoldReleased: (message: string) => void;
  bookingStep: React.ReactNode;
  confirmed: React.ReactNode;
}) {
  const [secondsRemaining, setSecondsRemaining] = useState<number | null>(null);
  const [releasePending, setReleasePending] = useState(false);
  const [releaseError, setReleaseError] = useState("");
  const expiresAt = new Date(reservation.expires_at).getTime();
  const eventClock = useMemo(
    () => createEventClock(eventTimezone),
    [eventTimezone],
  );
  const holdExpired = secondsRemaining === 0;

  async function releaseHold() {
    if (!reservation.idempotency_key) return;
    setReleasePending(true);
    setReleaseError("");
    try {
      const response = await fetch("/api/reservations", {
        method: "DELETE",
        headers: {
          "Content-Type": "application/json",
          "Idempotency-Key": reservation.idempotency_key,
        },
        body: JSON.stringify({ event_id: reservation.event_id, reservation_id: reservation.id }),
      });
      if (!response.ok) {
        setReleaseError("Unable to cancel the hold right now. Please try again.");
        return;
      }
      const status = await readReservationReleaseStatus(response);
      if (!status) {
        setReleaseError("Unable to cancel the hold right now. Please try again.");
        return;
      }
      onHoldReleased(status === "released"
        ? "Hold canceled. You can select tickets again."
        : "This hold had already expired. Ticket availability has been refreshed.");
    } catch {
      setReleaseError("Unable to cancel the hold right now. Please try again.");
    } finally {
      setReleasePending(false);
    }
  }

  useEffect(() => {
    if (!Number.isFinite(expiresAt)) return;

    const updateRemainingTime = () => {
      setSecondsRemaining(Math.max(0, Math.ceil((expiresAt - Date.now()) / 1_000)));
    };

    updateRemainingTime();
    const intervalId = window.setInterval(updateRemainingTime, 1_000);
    return () => window.clearInterval(intervalId);
  }, [expiresAt]);

  return (
    <div className={styles.checkoutSuccess}>
      <div role="status">
        <strong>{holdExpired ? "Hold expired" : confirmed ? "Booking confirmed" : "Spots held"}</strong>
        <span>
          {reservation.quantity} ticket{reservation.quantity === 1 ? "" : "s"} ·{" "}
          {formatMoney(reservation.amount_due.amount, reservation.amount_due.currency)}
        </span>
      </div>
      {confirmed ?? null}
      {!confirmed && secondsRemaining !== null && !holdExpired && (
        <span className={styles.holdTimer} role="timer">
          Time left {formatRemainingTime(secondsRemaining)}
        </span>
      )}
      {holdExpired && (
        <>
          <p className={styles.holdExpired} role="status">
            This hold window has ended. Ticket availability is checked again before another hold is
            created.
          </p>
          <Button
            className={styles.recheckTicketsButton}
            onClick={onCheckAvailability}
            type="button"
            variant="outline"
          >
            Check availability again
          </Button>
        </>
      )}
      {!confirmed && !holdExpired && reservation.idempotency_key && (
        <Button disabled={releasePending} onClick={releaseHold} type="button" variant="outline">
          {releasePending ? "Releasing hold…" : releaseError ? "Retry release" : "Give up my spots"}
        </Button>
      )}
      {confirmed ? null : holdExpired ? null : bookingStep}
      {releaseError && <p role="alert">{releaseError}</p>}
      {/* The organizer and support need a reference; nobody deciding whether to
          attend does. Keeping it collapsed is what stops an internal identifier
          reading as the most important thing on the screen. */}
      <details className={styles.supportDetails}>
        <summary>{`Need help with this booking?`}</summary>
        <p>
          Quote reference <code>{reservation.id}</code>
          {eventClock.usesUtcFallback ? " (UTC)" : ` · ${eventClock.label}`}
        </p>
      </details>
      {confirmed ? null : (
        <span>No payment is taken. Confirming books your spots; entry tickets follow.</span>
      )}
    </div>
  );
}

export function TicketCheckout({
  ticketTypes,
  eventTimezone,
  eventId,
  formFields = [],
}: {
  ticketTypes: TicketType[];
  eventTimezone: string;
  eventId: string;
  formFields?: components["schemas"]["RegistrationFormField"][];
}) {
  const router = useRouter();
  const [ticketTypeId, setTicketTypeId] = useState<string>(
    ticketTypes.find((ticket) => ticket.availability === "available")?.id ?? "",
  );
  const [quantityInput, setQuantityInput] = useState("1");
  const [currentReservation, setCurrentReservation] = useState<{
    eventId: string;
    reservation: Reservation & { idempotency_key?: string };
  } | null>(null);
  const storedConfirmation = useSyncExternalStore(
    subscribeToReservationConfirmation,
    () => getReservationConfirmationSnapshot(eventId),
    () => "",
  );
  const reservation = (currentReservation?.eventId === eventId ? currentReservation.reservation : null)
    ?? parseReservationConfirmation(storedConfirmation, eventId);
  const [error, setError] = useState("");
  const [reservationNotice, setReservationNotice] = useState("");
  const [supportReference, setSupportReference] = useState("");
  const [confirmedOrder, setConfirmedOrder] = useState<ConfirmedOrder | null>(null);
  const [isPending, startTransition] = useTransition();
  const retryRef = useRef<{ fingerprint: string; key: string } | null>(null);
  const selectedTicket = ticketTypes.find((ticket) => ticket.id === ticketTypeId);
  const hasAvailableTickets = ticketTypes.some((ticket) => ticket.availability === "available");
  const quantity = parseReservationQuantity(quantityInput);

  function clearCheckoutError() {
    setError("");
    setSupportReference("");
  }

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!selectedTicket || quantity === null) return;
    const fingerprint = JSON.stringify([eventId, ticketTypeId, quantity]);
    if (!retryRef.current || retryRef.current.fingerprint !== fingerprint) {
      retryRef.current = loadReservationRetry(reservationRetryStorage(), fingerprint) ?? {
        fingerprint,
        key: newIdempotencyKey(),
      };
      saveReservationRetry(reservationRetryStorage(), retryRef.current);
    }
    const key = retryRef.current.key;

    startTransition(async () => {
      clearCheckoutError();
      setReservationNotice("");
      setCurrentReservation(null);
      try {
        const response = await fetch("/api/reservations", {
          method: "POST",
          headers: { "Content-Type": "application/json", "Idempotency-Key": key },
          body: JSON.stringify({
            event_id: eventId,
            ticket_type_id: ticketTypeId,
            quantity,
          }),
        });
        if (response.status === 429) {
          setError("This network has sent too many reservation requests recently. Please wait a minute, then try again.");
          const requestId = getSupportReference(response.headers.get("X-Request-ID"));
          if (requestId) setSupportReference(requestId);
          return;
        }
        const body = await readReservationResponseBody(response);
        if (!response.ok) {
          const error = body?.error && typeof body.error === "object" && !Array.isArray(body.error)
            ? body.error as Record<string, unknown>
            : null;
          const message = error?.code === "INVENTORY_UNAVAILABLE"
            ? "That quantity is no longer available. Reduce the quantity or choose another ticket type, then try again."
            : error?.code === "SALE_CLOSED"
              ? "Reservations are closed for this ticket type. Choose another available ticket or check back later."
              : typeof body?.detail === "string"
                ? body.detail
                : response.status >= 500
                  ? "The reservation service returned an unexpected response. You can safely retry this request."
                  : "The reservation could not be completed. Check the ticket details and try again.";
          setError(message);
          if (response.status >= 500) {
            const requestId = getSupportReference(
              response.headers.get("X-Request-ID"),
              body?.request_id,
            );
            if (requestId) setSupportReference(requestId);
          }
          return;
        }
        const confirmation = body
          ? parseReservationConfirmation(JSON.stringify({ ...body, idempotency_key: key }), eventId)
          : null;
        if (!confirmation) {
          setError("The reservation service returned an invalid confirmation. Retry this request to check its saved result.");
          const requestId = getSupportReference(response.headers.get("X-Request-ID"), body?.request_id);
          if (requestId) setSupportReference(requestId);
          return;
        }
        saveReservationConfirmation(reservationRetryStorage(), eventId, confirmation);
        clearReservationRetry(reservationRetryStorage(), key);
        retryRef.current = null;
        setCurrentReservation({ eventId, reservation: confirmation });
      } catch {
        setError("The reservation service could not be reached. You can safely retry this request.");
      }
    });
  }

  return (
    <>
      {reservationNotice && <p role="status">{reservationNotice}</p>}
      {!reservation && (
        <fieldset aria-busy={isPending} className={styles.ticketList} disabled={isPending}>
          <legend className={styles.ticketLegend}>Choose a ticket type</legend>
          {ticketTypes.map((ticket) => {
            const unavailable = ticket.availability !== "available";
            const selected = ticketTypeId === ticket.id;

            return (
              <label
                className={`${styles.ticketOption} ${selected ? styles.ticketOptionSelected : ""} ${unavailable ? styles.ticketOptionDisabled : ""} ${isPending ? styles.ticketOptionPending : ""}`}
                key={ticket.id}
              >
                <input
                  checked={selected}
                  className={styles.ticketRadio}
                  disabled={unavailable || isPending}
                  name="ticket_type_id"
                  onChange={() => {
                    setTicketTypeId(ticket.id);
                    clearCheckoutError();
                  }}
                  type="radio"
                  value={ticket.id}
                />
                <span className={styles.ticketCopy}>
                  <strong>{ticket.name}</strong>
                  <span>{ticket.description ?? "Ticket details will be announced by the organizer."}</span>
                  <small>
                    {ticket.availability === "sold_out"
                      ? "Sold out"
                      : ticket.availability === "not_on_sale"
                        ? "Sales are not open"
                        : selected
                          ? "Selected · availability confirmed on reservation"
                          : "Available"}
                  </small>
                </span>
                <strong className={styles.ticketPrice}>
                  {formatMoney(ticket.price.amount, ticket.price.currency)}
                </strong>
              </label>
            );
          })}
        </fieldset>
      )}

      {reservation ? (
        <ReservationConfirmation
          eventTimezone={eventTimezone}
          onCheckAvailability={() => {
            clearReservationConfirmation(reservationRetryStorage(), eventId);
            setCurrentReservation(null);
            clearCheckoutError();
            router.refresh();
          }}
          onHoldReleased={(message) => {
            clearReservationConfirmation(reservationRetryStorage(), eventId);
            if (reservation.idempotency_key) {
              clearReservationRetry(reservationRetryStorage(), reservation.idempotency_key);
            }
            retryRef.current = null;
            setCurrentReservation(null);
            setReservationNotice(message);
            clearCheckoutError();
            router.refresh();
          }}
          reservation={reservation}
          bookingStep={
            confirmedOrder ? null : (
              <BookingDetails
                currency={reservation.amount_due.currency}
                eventId={eventId}
                expiresAt={reservation.expires_at}
                formFields={formFields}
                onConfirmed={(order) => setConfirmedOrder(order)}
                onExpired={() => {
                  clearReservationConfirmation(reservationRetryStorage(), eventId);
                  setCurrentReservation(null);
                  setReservationNotice("Your hold ended before the booking was confirmed. Choose your tickets again.");
                  router.refresh();
                }}
                reservationId={reservation.id}
                total={reservation.amount_due.amount}
              />
            )
          }
          confirmed={confirmedOrder ? <TicketWallet order={confirmedOrder} /> : null}
        />
      ) : hasAvailableTickets && selectedTicket ? (
        <form aria-busy={isPending} className={styles.checkoutForm} onSubmit={submit}>
          <label className={styles.checkoutField}>
            <span>Quantity</span>
            <Input
              max={MAX_RESERVATION_QUANTITY}
              min={1}
              aria-describedby={quantity === null ? "quantity-error" : undefined}
              aria-invalid={quantity === null}
              disabled={isPending}
              id="ticket-quantity"
              onChange={(event) => {
                setQuantityInput(event.target.value);
                clearCheckoutError();
              }}
              required
              type="number"
              value={quantityInput}
            />
          </label>
          {quantity === null && (
            <p className={styles.quantityError} id="quantity-error">
              Enter a whole number from 1 to 100.
            </p>
          )}
          {quantity !== null && (
            <p aria-live="polite" className={styles.reservationTotal}>
              Reservation total <strong>{formatMoney(multiplyAmount(selectedTicket.price.amount, quantity), selectedTicket.price.currency)}</strong>
            </p>
          )}
          <Button className={styles.ticketButton} disabled={isPending || quantity === null || !selectedTicket} type="submit">
            {isPending ? "Holding your tickets…" : "Reserve tickets"}<span aria-hidden="true">↗</span>
          </Button>
          <p className={styles.ticketFootnote}>You can cancel this hold at any time before it expires.</p>
          <div className={styles.checkoutResult}>
            {error && (
              <div className={styles.checkoutError} role="alert">
                <span>{error}</span>
                {supportReference && (
                  <span>Support reference: <code>{supportReference}</code></span>
                )}
              </div>
            )}
          </div>
        </form>
      ) : (
        <p className={styles.noTickets} role="status">
          {ticketTypes.length === 0
            ? "No ticket types are available for this event yet."
            : "Ticket reservations aren’t available right now. Please check back with the organizer."}
        </p>
      )}
    </>
  );
}

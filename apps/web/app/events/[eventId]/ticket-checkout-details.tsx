"use client";

import { useEffect, useState } from "react";
import type { components } from "@zeno/api-client";
import { QRCodeSVG } from "qrcode.react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import styles from "./event-page.module.css";

type Field = components["schemas"]["RegistrationFormField"];
export type ConfirmedOrder = components["schemas"]["ConfirmedOrder"];

const MONEY = new Intl.NumberFormat("en");

function formatMoney(amount: string, currency: string) {
  const [whole, fraction] = amount.split(".");
  return `${currency} ${MONEY.format(BigInt(whole))}${fraction ? `.${fraction}` : ""}`;
}

function FieldInput({
  field,
  value,
  onChange,
  invalid,
}: {
  field: Field;
  value: string;
  onChange: (next: string) => void;
  invalid?: string;
}) {
  const id = `answer-${field.field_key}`;
  const describedBy = invalid ? `${id}-error` : undefined;

  if (field.field_type === "single_choice" || field.field_type === "multi_choice") {
    const multiple = field.field_type === "multi_choice";
    const selected = multiple ? value.split("\n").filter(Boolean) : value ? [value] : [];
    return (
      <fieldset className={styles.answerGroup} aria-describedby={describedBy} aria-invalid={invalid ? true : undefined}>
        <legend>
          {field.label}
          {field.required ? <b aria-hidden="true"> *</b> : null}
        </legend>
        {field.help_text ? <p className={styles.answerHelp}>{field.help_text}</p> : null}
        {field.options.map((option) => {
          const checked = selected.includes(option);
          return (
            <label className={styles.answerChoice} key={option}>
              <input
                checked={checked}
                name={multiple ? `${field.field_key}[]` : field.field_key}
                onChange={(event) => {
                  if (!multiple) {
                    onChange(event.target.checked ? option : "");
                    return;
                  }
                  const next = event.target.checked
                    ? [...selected, option]
                    : selected.filter((item) => item !== option);
                  onChange(next.join("\n"));
                }}
                type={multiple ? "checkbox" : "radio"}
                value={option}
              />
              <span>{option}</span>
            </label>
          );
        })}
        {invalid ? (
          <p className={styles.answerError} id={`${id}-error`} role="alert">
            {invalid}
          </p>
        ) : null}
      </fieldset>
    );
  }

  if (field.field_type === "long_text") {
    return (
      <label className={styles.answerField} htmlFor={id}>
        <span>
          {field.label}
          {field.required ? <b aria-hidden="true"> *</b> : null}
        </span>
        {field.help_text ? <small>{field.help_text}</small> : null}
        <textarea
          aria-describedby={describedBy}
          aria-invalid={invalid ? true : undefined}
          id={id}
          maxLength={2000}
          onChange={(event) => onChange(event.target.value)}
          rows={3}
          value={value}
        />
        {invalid ? (
          <span className={styles.answerError} id={`${id}-error`} role="alert">
            {invalid}
          </span>
        ) : null}
      </label>
    );
  }

  const inputType =
    field.field_type === "email"
      ? "email"
      : field.field_type === "phone"
        ? "tel"
        : field.field_type === "number"
          ? "number"
          : field.field_type === "checkbox"
            ? "checkbox"
            : "text";

  return (
    <label className={styles.answerField} htmlFor={id}>
      <span>
        {field.label}
        {field.required ? <b aria-hidden="true"> *</b> : null}
      </span>
      {field.help_text ? <small>{field.help_text}</small> : null}
      {field.field_type === "checkbox" ? (
        <input
          checked={value === "true"}
          id={id}
          onChange={(event) => onChange(event.target.checked ? "true" : "")}
          type="checkbox"
        />
      ) : (
        <input
          aria-describedby={describedBy}
          aria-invalid={invalid ? true : undefined}
          id={id}
          maxLength={field.field_type === "email" || field.field_type === "phone" ? 254 : 500}
          onChange={(event) => onChange(event.target.value)}
          type={inputType}
          value={value}
        />
      )}
      {invalid ? (
        <span className={styles.answerError} id={`${id}-error`} role="alert">
          {invalid}
        </span>
      ) : null}
    </label>
  );
}

/**
 * The details step shown after a hold exists.
 *
 * Kept separate from the hold timer and the ticket picker so each screen answers
 * one question: what to book, who it is for, and what we still owe you.
 */
export function BookingDetails({
  formFields,
  expiresAt,
  total,
  currency,
  eventId,
  reservationId,
  onConfirmed,
  onExpired,
}: {
  formFields: Field[];
  expiresAt: string;
  total: string;
  currency: string;
  eventId: string;
  reservationId: string;
  onConfirmed: (order: ConfirmedOrder) => void;
  onExpired: () => void;
}) {
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [submitting, setSubmitting] = useState(false);
  const [failure, setFailure] = useState("");

  const remaining = useRemainingSeconds(expiresAt);

  async function submit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmitting(true);
    setFailure("");
    setErrors({});

    try {
      const response = await fetch("/api/reservations/confirm", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          event_id: eventId,
          reservation_id: reservationId,
          purchaser_name: name,
          purchaser_email: email,
          answers: Object.entries(answers)
            .filter(([, value]) => value !== "")
            .flatMap(([field_key, value]) =>
              value.includes("\n")
                ? value
                    .split("\n")
                    .filter(Boolean)
                    .map((entry) => ({ field_key, value: entry }))
                : [{ field_key, value }],
            ),
        }),
      });
      const body = await response.json().catch(() => null);

      if (!response.ok) {
        const code = body?.error?.code;
        if (code === "RESERVATION_EXPIRED") {
          onExpired();
          return;
        }
        setFailure(
          code === "INVALID_REGISTRATION_ANSWERS"
            ? "Check the highlighted answers and try again."
            : code === "SERVICE_UNAVAILABLE" || code === "ORDERING_UNAVAILABLE"
              ? "We couldn't complete the booking. Your spots are still held, so you can retry."
              : "We couldn't complete the booking. Check your details and try again.",
        );
        return;
      }

      onConfirmed(body as ConfirmedOrder);
    } catch {
      setFailure(
        "We couldn't reach the booking service. Your spots are still held, so you can retry.",
      );
    } finally {
      setSubmitting(false);
    }
  }

  const minutes = Math.floor(remaining / 60);
  const seconds = remaining % 60;

  return (
    <form className={styles.bookingForm} onSubmit={submit}>
      <p className={styles.bookingTotal}>
        Total <strong>{formatMoney(total, currency)}</strong>
      </p>

      {remaining > 0 ? (
        <p className={styles.bookingTimer} role="timer">
          Held for {minutes}:{seconds.toString().padStart(2, "0")}
        </p>
      ) : null}

      <label className={styles.answerField} htmlFor="booking-name">
        <span>
          Booking name <b aria-hidden="true">*</b>
        </span>
        <small>Who this booking is for, for example a company or society.</small>
        <Input
          autoComplete="organization"
          id="booking-name"
          maxLength={120}
          onChange={(event) => setName(event.target.value)}
          required
          value={name}
        />
      </label>

      <label className={styles.answerField} htmlFor="booking-email">
        <span>
          Email <b aria-hidden="true">*</b>
        </span>
        <small>Where we would send the tickets.</small>
        <Input
          autoComplete="email"
          id="booking-email"
          maxLength={254}
          onChange={(event) => setEmail(event.target.value)}
          required
          type="email"
          value={email}
        />
      </label>

      {formFields.length ? (
        <div className={styles.answerGroup}>
          <p className={styles.answerGroupTitle}>{`Questions from the organizer`}</p>
          {formFields.map((field) => (
            <FieldInput
              field={field}
              invalid={errors[field.field_key]}
              key={field.field_key}
              onChange={(value) => setAnswers((current) => ({ ...current, [field.field_key]: value }))}
              value={answers[field.field_key] ?? ""}
            />
          ))}
        </div>
      ) : null}

      {failure ? (
        <p className={styles.checkoutError} role="alert">
          {failure}
        </p>
      ) : null}

      <Button disabled={submitting || remaining === 0} size="lg" type="submit">
        {submitting ? "Confirming…" : "Confirm booking"}
      </Button>
      <p className={styles.ticketFootnote}>
        No payment is taken. Confirming issues one entry ticket per guest, each with its own QR
        code, so your group can arrive whenever they like.
      </p>
    </form>
  );
}

/** Countdown for the hold, clamped at zero. */
function useRemainingSeconds(expiresAt: string) {
  const [remaining, setRemaining] = useState(() =>
    Math.max(0, Math.ceil((new Date(expiresAt).getTime() - Date.now()) / 1000)),
  );
  useEffect(() => {
    const id = window.setInterval(() => {
      setRemaining(
        Math.max(0, Math.ceil((new Date(expiresAt).getTime() - Date.now()) / 1000)),
      );
    }, 1000);
    return () => window.clearInterval(id);
  }, [expiresAt]);
  return remaining;
}

export function TicketWallet({ order }: { order: ConfirmedOrder }) {
  return (
    <div className={styles.wallet}>
      <p className={styles.walletHeading}>
        <strong>{order.quantity === 1 ? "1 ticket" : `${order.quantity} tickets`}</strong>
        <span>{formatMoney(order.total_amount.amount, order.total_amount.currency)}</span>
      </p>
      <p className={styles.walletNote}>
        Show any one of these at the door. Each ticket admits one guest, so your group can arrive
        whenever they like.
      </p>
      <ul className={styles.walletList}>
        {order.tickets.map((ticket) => (
          <li className={styles.walletItem} key={ticket.id}>
            <span className={styles.walletSequence}>
              Ticket {ticket.sequence} of {order.quantity}
            </span>
            <div className={styles.walletCode}>
              <QRCodeSVG
                className={styles.walletQr}
                level="M"
                marginSize={4}
                size={168}
                title={`QR code for ticket ${ticket.sequence} of ${order.quantity}`}
                value={ticket.credential}
              />
              <p className={styles.walletManual}>
                <span>Or read this code out</span>
                <code className={styles.walletReference}>{ticket.reference}</code>
              </p>
            </div>
          </li>
        ))}
      </ul>
      <details className={styles.supportDetails}>
        <summary>{`Need help with this booking?`}</summary>
        <p>
          Quote reference <code>{order.id}</code>
        </p>
      </details>
      {order.answers.length ? (
        <details className={styles.walletAnswers}>
          <summary>{`What you told the organizer`}</summary>
          <dl>
            {order.answers.map((answer) => (
              <div key={answer.field_key}>
                <dt>{answer.field_label}</dt>
                <dd>{answer.value.split("\n").join(", ")}</dd>
              </div>
            ))}
          </dl>
        </details>
      ) : null}
    </div>
  );
}
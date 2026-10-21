export type ReservationRetry = {
  fingerprint: string;
  key: string;
};

export type ReservationConfirmation = {
  id: string;
  event_id: string;
  ticket_type_id: string;
  quantity: number;
  amount_due: { amount: string; currency: string };
  status: "held";
  expires_at: string;
  idempotency_key?: string;
};

type RetryStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

const STORAGE_KEY = "zeno:reservation-retry";
const RESERVATION_STORAGE_EVENT = "zeno:reservation-storage-change";
const IDEMPOTENCY_KEY_PATTERN = /^[A-Za-z0-9_-]{22,128}$/;
const PUBLIC_ID_PATTERN = /^zeno_[A-Za-z0-9]{16,32}$/;
const MAX_SAVED_RETRIES = 20;

function isReservationRetry(value: unknown): value is ReservationRetry {
  if (!value || typeof value !== "object") return false;
  const retry = value as Partial<ReservationRetry>;
  return typeof retry.fingerprint === "string"
    && retry.fingerprint.length > 0
    && retry.fingerprint.length <= 256
    && typeof retry.key === "string"
    && IDEMPOTENCY_KEY_PATTERN.test(retry.key);
}

function readReservationRetries(storage: RetryStorage): ReservationRetry[] {
  const stored = storage.getItem(STORAGE_KEY);
  if (!stored) return [];

  try {
    const parsed: unknown = JSON.parse(stored);
    // Read the original single-key format so a pending request survives this update.
    if (isReservationRetry(parsed)) return [parsed];
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isReservationRetry).slice(-MAX_SAVED_RETRIES);
  } catch {
    return [];
  }
}

function writeReservationRetries(storage: RetryStorage, retries: ReservationRetry[]) {
  if (retries.length === 0) {
    storage.removeItem(STORAGE_KEY);
    return;
  }
  storage.setItem(STORAGE_KEY, JSON.stringify(retries.slice(-MAX_SAVED_RETRIES)));
}

function confirmationStorageKey(eventId: string) {
  return `zeno:reservation-confirmation:${eventId}`;
}

function notifyReservationStorageChanged() {
  if (typeof window !== "undefined") {
    window.dispatchEvent(new Event(RESERVATION_STORAGE_EVENT));
  }
}

export function subscribeToReservationConfirmation(onChange: () => void) {
  if (typeof window === "undefined") return () => {};

  window.addEventListener(RESERVATION_STORAGE_EVENT, onChange);
  return () => window.removeEventListener(RESERVATION_STORAGE_EVENT, onChange);
}

export function getReservationConfirmationSnapshot(eventId: string) {
  if (typeof window === "undefined" || !PUBLIC_ID_PATTERN.test(eventId)) return "";

  try {
    return window.sessionStorage.getItem(confirmationStorageKey(eventId)) ?? "";
  } catch {
    return "";
  }
}

export function loadReservationRetry(
  storage: RetryStorage | null,
  fingerprint: string,
): ReservationRetry | null {
  if (!storage) return null;

  try {
    return readReservationRetries(storage).find((retry) => retry.fingerprint === fingerprint) ?? null;
  } catch {
    return null;
  }
}

export function saveReservationRetry(
  storage: RetryStorage | null,
  retry: ReservationRetry,
) {
  if (!storage) return;

  try {
    const retries = readReservationRetries(storage).filter((saved) => saved.fingerprint !== retry.fingerprint);
    retries.push(retry);
    writeReservationRetries(storage, retries);
  } catch {
    // The in-memory key still makes retries safe while this page remains open.
  }
}

export function clearReservationRetry(storage: RetryStorage | null, key: string) {
  if (!storage) return;

  try {
    writeReservationRetries(storage, readReservationRetries(storage).filter((retry) => retry.key !== key));
  } catch {
    // A malformed saved retry must never prevent a completed reservation.
  }
}

export function loadReservationConfirmation(
  storage: RetryStorage | null,
  eventId: string,
): ReservationConfirmation | null {
  if (!storage || !PUBLIC_ID_PATTERN.test(eventId)) return null;

  try {
    return parseReservationConfirmation(
      storage.getItem(confirmationStorageKey(eventId)) ?? "",
      eventId,
    );
  } catch {
    return null;
  }
}

export function parseReservationConfirmation(serialized: string, eventId: string) {
  if (!serialized || !PUBLIC_ID_PATTERN.test(eventId)) return null;

  try {
    const reservation = JSON.parse(serialized) as Partial<ReservationConfirmation>;
    if (
      typeof reservation.id !== "string" ||
      !PUBLIC_ID_PATTERN.test(reservation.id) ||
      reservation.event_id !== eventId ||
      typeof reservation.ticket_type_id !== "string" ||
      !PUBLIC_ID_PATTERN.test(reservation.ticket_type_id) ||
      typeof reservation.quantity !== "number" ||
      !Number.isInteger(reservation.quantity) ||
      reservation.quantity < 1 ||
      reservation.quantity > 100 ||
      typeof reservation.amount_due !== "object" ||
      reservation.amount_due === null ||
      typeof reservation.amount_due.amount !== "string" ||
      !/^\d+(?:\.\d+)?$/.test(reservation.amount_due.amount) ||
      typeof reservation.amount_due.currency !== "string" ||
      !/^[A-Z]{3}$/.test(reservation.amount_due.currency) ||
      reservation.status !== "held" ||
      typeof reservation.expires_at !== "string" ||
      !Number.isFinite(Date.parse(reservation.expires_at)) ||
      (reservation.idempotency_key !== undefined &&
        (typeof reservation.idempotency_key !== "string" ||
          !IDEMPOTENCY_KEY_PATTERN.test(reservation.idempotency_key)))
    ) {
      return null;
    }

    return reservation as ReservationConfirmation;
  } catch {
    return null;
  }
}

export function saveReservationConfirmation(
  storage: RetryStorage | null,
  eventId: string,
  reservation: ReservationConfirmation,
) {
  if (!storage || eventId !== reservation.event_id) return;

  try {
    storage.setItem(confirmationStorageKey(eventId), JSON.stringify(reservation));
    notifyReservationStorageChanged();
  } catch {
    // Keep the confirmation in component state when session storage is unavailable.
  }
}

export function clearReservationConfirmation(storage: RetryStorage | null, eventId: string) {
  if (!storage || !PUBLIC_ID_PATTERN.test(eventId)) return;

  try {
    storage.removeItem(confirmationStorageKey(eventId));
    notifyReservationStorageChanged();
  } catch {
    // A storage failure must not prevent checking availability again.
  }
}

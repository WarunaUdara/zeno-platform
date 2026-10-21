import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  clearReservationConfirmation,
  clearReservationRetry,
  parseReservationConfirmation,
  loadReservationConfirmation,
  loadReservationRetry,
  saveReservationConfirmation,
  saveReservationRetry,
} from "./reservation-retry";

class MemoryStorage {
  private values = new Map<string, string>();

  getItem(key: string) {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string) {
    this.values.set(key, value);
  }

  removeItem(key: string) {
    this.values.delete(key);
  }
}

describe("reservation retry persistence", () => {
  it("reuses a valid key only for the same request after a reload", () => {
    const storage = new MemoryStorage();
    const retry = {
      fingerprint: JSON.stringify(["zeno_event", "zeno_ticket", 2]),
      key: "abcdefghijklmnopqrstu_",
    };

    saveReservationRetry(storage, retry);

    assert.deepEqual(loadReservationRetry(storage, retry.fingerprint), retry);
    assert.equal(loadReservationRetry(storage, JSON.stringify(["zeno_event", "zeno_ticket", 3])), null);
  });

  it("clears only the completed request's key", () => {
    const storage = new MemoryStorage();
    const retry = { fingerprint: "request", key: "abcdefghijklmnopqrstu_" };
    saveReservationRetry(storage, retry);
    saveReservationRetry(storage, { fingerprint: "new request", key: "zyxwvutsrqponmlkjihgfe" });

    clearReservationRetry(storage, retry.key);

    assert.deepEqual(
      loadReservationRetry(storage, "new request"),
      { fingerprint: "new request", key: "zyxwvutsrqponmlkjihgfe" },
    );
  });

  it("preserves retry keys for multiple request variants in the same tab", () => {
    const storage = new MemoryStorage();
    const first = { fingerprint: "event-a/ticket-a/1", key: "abcdefghijklmnopqrstu_" };
    const second = { fingerprint: "event-a/ticket-b/2", key: "zyxwvutsrqponmlkjihgfe" };
    saveReservationRetry(storage, first);
    saveReservationRetry(storage, second);

    assert.deepEqual(loadReservationRetry(storage, first.fingerprint), first);
    assert.deepEqual(loadReservationRetry(storage, second.fingerprint), second);
    clearReservationRetry(storage, first.key);
    assert.equal(loadReservationRetry(storage, first.fingerprint), null);
    assert.deepEqual(loadReservationRetry(storage, second.fingerprint), second);
  });

  it("continues a pending request saved in the original single-key format", () => {
    const storage = new MemoryStorage();
    const retry = { fingerprint: "request", key: "abcdefghijklmnopqrstu_" };
    storage.setItem("zeno:reservation-retry", JSON.stringify(retry));

    assert.deepEqual(loadReservationRetry(storage, retry.fingerprint), retry);
    saveReservationRetry(storage, { fingerprint: "another request", key: "zyxwvutsrqponmlkjihgfe" });
    assert.deepEqual(loadReservationRetry(storage, retry.fingerprint), retry);
  });

  it("ignores malformed or invalid keys", () => {
    const storage = new MemoryStorage();
    storage.setItem("zeno:reservation-retry", JSON.stringify({ fingerprint: "request", key: "short" }));

    assert.equal(loadReservationRetry(storage, "request"), null);
  });

  it("restores an unexpired hold for the same event after a reload", () => {
    const storage = new MemoryStorage();
    const eventId = "zeno_0123456789abcdef";
    const reservation = {
      id: "zeno_fedcba9876543210",
      event_id: eventId,
      ticket_type_id: "zeno_1234567890abcdef",
      quantity: 2,
      amount_due: { amount: "2500", currency: "LKR" },
      status: "held" as const,
      expires_at: "2099-01-01T00:00:00Z",
      idempotency_key: "abcdefghijklmnopqrstu_",
    };

    saveReservationConfirmation(storage, eventId, reservation);

    assert.deepEqual(loadReservationConfirmation(storage, eventId), reservation);
    assert.deepEqual(parseReservationConfirmation(JSON.stringify(reservation), eventId), reservation);
    assert.equal(loadReservationConfirmation(storage, "zeno_abcdef0123456789"), null);
    clearReservationConfirmation(storage, eventId);
    assert.equal(loadReservationConfirmation(storage, eventId), null);
  });

  it("ignores a malformed reservation confirmation", () => {
    const storage = new MemoryStorage();
    const eventId = "zeno_0123456789abcdef";
    storage.setItem(
      `zeno:reservation-confirmation:${eventId}`,
      JSON.stringify({ id: "bad", event_id: eventId }),
    );

    assert.equal(loadReservationConfirmation(storage, eventId), null);
  });

  it("keeps older confirmations readable but rejects malformed release credentials", () => {
    const eventId = "zeno_0123456789abcdef";
    const confirmation = {
      id: "zeno_fedcba9876543210",
      event_id: eventId,
      ticket_type_id: "zeno_1234567890abcdef",
      quantity: 1,
      amount_due: { amount: "1250", currency: "LKR" },
      status: "held",
      expires_at: "2099-01-01T00:00:00Z",
    };

    assert.ok(parseReservationConfirmation(JSON.stringify(confirmation), eventId));
    assert.equal(
      parseReservationConfirmation(JSON.stringify({
        ...confirmation,
        idempotency_key: "short",
      }), eventId),
      null,
    );
  });
});

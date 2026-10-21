import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { ticketCreateFingerprint } from "./ticket-create-retry";

function formData(overrides: Record<string, string> = {}) {
  const values = {
    name: "General admission",
    description: "",
    amount: "25.00",
    currency: "USD",
    capacity: "100",
    timezone: "America/New_York",
    sale_starts_at: "2026-01-15T19:30",
    sale_ends_at: "2026-01-15T21:00",
    ...overrides,
  };
  const data = new FormData();
  for (const [key, value] of Object.entries(values)) data.append(key, value);
  return data;
}

describe("ticket-create idempotency fingerprint", () => {
  it("matches equivalent normalized requests", async () => {
    const first = await ticketCreateFingerprint("event-a", "org-a", formData());
    const normalized = await ticketCreateFingerprint("event-a", "org-a", formData({
      name: " General admission ",
      currency: "usd",
      capacity: "0100",
    }));

    assert.equal(normalized, first);
  });

  it("uses a new fingerprint when a retry changes the request", async () => {
    const first = await ticketCreateFingerprint("event-a", "org-a", formData());
    const changed = await ticketCreateFingerprint("event-a", "org-a", formData({ amount: "30.00" }));

    assert.notEqual(changed, first);
  });

  it("fingerprints equivalent instants the same across timezones", async () => {
    const first = await ticketCreateFingerprint("event-a", "org-a", formData());
    const changedZone = await ticketCreateFingerprint("event-a", "org-a", formData({
      timezone: "Asia/Colombo",
      sale_starts_at: "2026-01-16T06:00",
      sale_ends_at: "2026-01-16T07:30",
    }));

    assert.equal(changedZone, first);
  });
});

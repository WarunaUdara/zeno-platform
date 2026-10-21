import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  clearEventCreateRetry,
  eventCreateFingerprint,
  loadEventCreateRetry,
  saveEventCreateRetry,
} from "./event-create-retry";

function formData(overrides: Record<string, string> = {}) {
  const values = {
    title: "Community design night",
    summary: "Talk and workshop",
    timezone: "America/New_York",
    starts_at: "2026-06-15T19:30",
    ends_at: "2026-06-15T21:00",
    venue_kind: "physical",
    venue_name: "Main Hall",
    city: "New York",
    country_code: "US",
    ...overrides,
  };
  const data = new FormData();
  for (const [key, value] of Object.entries(values)) data.append(key, value);
  return data;
}

function storage() {
  const values = new Map<string, string>();
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
  };
}

describe("event-create retry fingerprint", () => {
  it("reuses a slug only for the same normalized event request", async () => {
    const first = await eventCreateFingerprint("org-a", formData());
    const equivalent = await eventCreateFingerprint("org-a", formData({
      title: " Community design night ",
      country_code: "us",
    }));
    const changed = await eventCreateFingerprint("org-a", formData({ title: "Community design night 2" }));

    assert.equal(equivalent, first);
    assert.notEqual(changed, first);
  });

  it("treats a timezone change as a different event request even when instants match", async () => {
    const eastern = await eventCreateFingerprint("org-a", formData());
    const colombo = await eventCreateFingerprint("org-a", formData({
      timezone: "Asia/Colombo",
      starts_at: "2026-06-16T05:00",
      ends_at: "2026-06-16T06:30",
    }));

    assert.notEqual(colombo, eastern);
  });

  it("persists a retry safely and ignores unavailable or invalid storage", async () => {
    const retry = { fingerprint: "request-fingerprint", slug: "community-design-night-12345678" };
    const session = storage();
    saveEventCreateRetry(session, "org-a", retry);
    assert.deepEqual(loadEventCreateRetry(session, "org-a", retry.fingerprint), retry);
    assert.equal(loadEventCreateRetry(session, "org-a", "different-request"), null);

    const blocked = {
      getItem() { throw new Error("storage blocked"); },
      setItem() { throw new Error("storage blocked"); },
      removeItem() { throw new Error("storage blocked"); },
    };
    saveEventCreateRetry(blocked, "org-a", retry);
    clearEventCreateRetry(blocked, "org-a");
    assert.equal(loadEventCreateRetry(blocked, "org-a", retry.fingerprint), null);
  });
});

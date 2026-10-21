import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { createEventClock } from "./event-time";

describe("event time formatting", () => {
  it("formats event dates in their published timezone", () => {
    const clock = createEventClock("Asia/Colombo");
    const time = clock.format("2099-01-01T04:30:00.000Z", {
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    });

    assert.equal(clock.usesUtcFallback, false);
    assert.equal(clock.label, "Asia/Colombo");
    assert.equal(time, "10:00");
  });

  it("falls back to clearly labeled UTC when event timezone data is invalid", () => {
    const clock = createEventClock("Not/A_Timezone");
    const time = clock.format("2099-01-01T04:30:00.000Z", {
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    });

    assert.equal(clock.usesUtcFallback, true);
    assert.equal(clock.label, "UTC");
    assert.equal(time, "04:30");
  });
});

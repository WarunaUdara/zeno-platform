import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { convertLocalDateTimesToTimeZone, isoToLocalDateTime, localDateTimeToIso } from "./local-datetime";

describe("local event times", () => {
  it("converts local event times using the selected IANA timezone", () => {
    assert.equal(localDateTimeToIso("2026-01-15T19:30", "America/New_York"), "2026-01-16T00:30:00.000Z");
    assert.equal(localDateTimeToIso("2026-01-16T10:00", "Asia/Colombo"), "2026-01-16T04:30:00.000Z");
  });

  it("chooses the earlier occurrence during a repeated daylight-saving hour", () => {
    assert.equal(localDateTimeToIso("2026-11-01T01:30", "America/New_York"), "2026-11-01T05:30:00.000Z");
  });

  it("formats saved instants back into the selected event timezone", () => {
    assert.equal(isoToLocalDateTime("2026-01-16T00:30:00.000Z", "America/New_York"), "2026-01-15T19:30");
  });

  it("preserves event instants when changing the selected timezone", () => {
    assert.deepEqual(
      convertLocalDateTimesToTimeZone(
        { startsAt: "2026-01-15T19:30", endsAt: "2026-01-15T21:00" },
        "America/New_York",
        "Asia/Colombo",
      ),
      { startsAt: "2026-01-16T06:00", endsAt: "2026-01-16T07:30" },
    );
  });

  it("preserves an empty optional end time and rejects invalid current values", () => {
    assert.deepEqual(
      convertLocalDateTimesToTimeZone(
        { startsAt: "2026-01-15T19:30", endsAt: "" },
        "America/New_York",
        "Asia/Colombo",
      ),
      { startsAt: "2026-01-16T06:00", endsAt: "" },
    );
    assert.equal(
      convertLocalDateTimesToTimeZone(
        { startsAt: "2026-03-08T02:30", endsAt: "" },
        "America/New_York",
        "Asia/Colombo",
      ),
      null,
    );
  });

  it("rejects nonexistent daylight-saving times and invalid input", () => {
    assert.equal(localDateTimeToIso("2026-03-08T02:30", "America/New_York"), null);
    assert.equal(localDateTimeToIso("2026-02-30T10:00", "UTC"), null);
    assert.equal(localDateTimeToIso("2026-01-01T10:00", "Not/A_Timezone"), null);
  });
});

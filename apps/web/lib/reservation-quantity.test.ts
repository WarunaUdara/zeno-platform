import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { MAX_RESERVATION_QUANTITY, parseReservationQuantity } from "./reservation-quantity";

describe("reservation quantity", () => {
  it("accepts whole quantities within the supported range", () => {
    assert.equal(parseReservationQuantity("1"), 1);
    assert.equal(parseReservationQuantity(String(MAX_RESERVATION_QUANTITY)), MAX_RESERVATION_QUANTITY);
  });

  it("rejects empty, fractional, unsafe, and out-of-range quantities", () => {
    for (const value of ["", "0", "101", "1.5", "-1", "1e2", "9007199254740992"]) {
      assert.equal(parseReservationQuantity(value), null, `expected ${value || "an empty value"} to be invalid`);
    }
  });
});

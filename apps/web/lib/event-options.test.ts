import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  DEFAULT_EVENT_TIMEZONE,
  DEFAULT_TICKET_CURRENCY,
  matchCurrencyOption,
  matchTimezoneOption,
} from "./event-options";

describe("event timezone options", () => {
  it("labels the home market first", () => {
    assert.equal(DEFAULT_EVENT_TIMEZONE, "Asia/Colombo");
    assert.equal(matchTimezoneOption("Asia/Colombo")?.label, "Colombo (GMT+5:30)");
  });

  it("keeps an unlisted zone rather than silently rewriting it", () => {
    // An event saved in a zone this build does not list must still open with its
    // own timezone; defaulting it elsewhere would move the start time.
    assert.equal(matchTimezoneOption("America/Sao_Paulo"), undefined);
  });
});

describe("ticket currency options", () => {
  it("defaults to the first market currency", () => {
    assert.equal(DEFAULT_TICKET_CURRENCY, "LKR");
    assert.equal(matchCurrencyOption("LKR")?.label, "LKR — Sri Lankan rupee");
  });

  it("does not invent an option for an unlisted currency", () => {
    assert.equal(matchCurrencyOption("JPY"), undefined);
  });
});
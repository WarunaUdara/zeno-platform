import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { canManageEvents, canReviewEventSetup, canRunCheckIn } from "./organizer-permissions";

describe("organizer role policy", () => {
  it("allows event managers to review and manage event setup", () => {
    for (const role of ["owner", "admin", "event_manager"]) {
      assert.equal(canReviewEventSetup(role), true, `${role} can review events`);
      assert.equal(canManageEvents(role), true, `${role} can manage events`);
    }
  });

  it("allows analysts to review setup without presenting mutation access", () => {
    assert.equal(canReviewEventSetup("analyst"), true);
    assert.equal(canManageEvents("analyst"), false);
  });

  it("does not imply event-setup access for check-in or unknown roles", () => {
    for (const role of ["check_in_manager", "attendee", "unknown"]) {
      assert.equal(canReviewEventSetup(role), false, `${role} cannot review setup`);
      assert.equal(canManageEvents(role), false, `${role} cannot manage events`);
    }
  });

  it("lets a check-in manager run the door without editing the event", () => {
    // The whole point of a narrower role: a volunteer is trusted to admit guests
    // and nothing else.
    assert.equal(canRunCheckIn("check_in_manager"), true);
    assert.equal(canManageEvents("check_in_manager"), false);
    for (const role of ["owner", "admin", "event_manager"]) {
      assert.equal(canRunCheckIn(role), true, `${role} can run the door`);
    }
    for (const role of ["analyst", "attendee", "unknown"]) {
      assert.equal(canRunCheckIn(role), false, `${role} cannot run the door`);
    }
  });
});

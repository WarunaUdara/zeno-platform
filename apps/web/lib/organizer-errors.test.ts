import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { organizerFormError, organizerListError } from "./organizer-errors";

describe("organizer API feedback", () => {
  it("does not claim form fields are highlighted when the API returns a page-level validation error", () => {
    assert.equal(organizerFormError(422, "fallback"), "Some details weren’t accepted. Review your entries and try again.");
  });

  it("explains that an invalid paginated list needs refreshing", () => {
    assert.equal(organizerListError(422, "fallback"), "This list changed while you were viewing it. Refresh the page to load the latest results.");
  });

  it("keeps shared session and missing-resource guidance consistent", () => {
    assert.equal(organizerFormError(401, "fallback"), "Your session ended. Sign in again to continue.");
    assert.equal(organizerListError(404, "fallback"), "This workspace or event is no longer available.");
  });
});

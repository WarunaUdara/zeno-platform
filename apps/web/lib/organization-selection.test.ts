import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { isValidOrganizationCursor, selectOrganization } from "./organization-selection";

const organizations = [
  { id: "org-a", name: "First workspace" },
  { id: "org-b", name: "Second workspace" },
];

describe("organizer workspace selection", () => {
  it("defaults to the first workspace when none is requested", () => {
    assert.equal(selectOrganization(organizations, undefined), organizations[0]);
  });

  it("selects the requested workspace", () => {
    assert.equal(selectOrganization(organizations, "org-b"), organizations[1]);
  });

  it("does not silently switch workspaces for an unknown id", () => {
    assert.equal(selectOrganization(organizations, "org-missing"), undefined);
  });
});

describe("organization page cursors", () => {
  it("allows an absent cursor and bounded URL-safe opaque values", () => {
    assert.equal(isValidOrganizationCursor(undefined), true);
    assert.equal(isValidOrganizationCursor(null), true);
    assert.equal(isValidOrganizationCursor("eyJjdXJzb3IiOiJwYWdlLTQifQ"), true);
    assert.equal(isValidOrganizationCursor("a".repeat(512)), true);
  });

  it("rejects malformed or oversized cursors before they reach the API", () => {
    assert.equal(isValidOrganizationCursor("a.b"), false);
    assert.equal(isValidOrganizationCursor("a".repeat(513)), false);
  });
});

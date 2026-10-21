import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  addOrganizationPage,
  createOrganizationPages,
  getNextOrganizationCursor,
  getOrganizationCursor,
} from "./organization-pages";

describe("organization pages", () => {
  it("keeps each loaded workspace attached to the cursor needed to select it", () => {
    const first = createOrganizationPages(
      [{ id: "org-1" }, { id: "org-2" }],
      null,
      "page-2",
    );
    const second = addOrganizationPage(
      first,
      "page-2",
      { items: [{ id: "org-3" }, { id: "org-4" }], next_cursor: null },
    );

    assert.deepEqual(second.items.map(({ id }) => id), ["org-1", "org-2", "org-3", "org-4"]);
    assert.equal(getOrganizationCursor(second, "org-1", "fallback"), null);
    assert.equal(getOrganizationCursor(second, "org-4", null), "page-2");
    assert.equal(getNextOrganizationCursor(second, null, null), "page-2");
    assert.equal(getNextOrganizationCursor(second, "page-2", "stale"), null);
  });

  it("deduplicates overlapping pages while updating current membership data", () => {
    const first = createOrganizationPages(
      [{ id: "org-1", name: "Old name" }, { id: "org-2", name: "Two" }],
      null,
      "page-2",
    );
    const second = addOrganizationPage(first, "page-2", {
      items: [{ id: "org-2", name: "Updated name" }, { id: "org-3", name: "Three" }],
      next_cursor: "page-3",
    });

    assert.deepEqual(second.items, [
      { id: "org-1", name: "Old name" },
      { id: "org-2", name: "Updated name" },
      { id: "org-3", name: "Three" },
    ]);
    assert.equal(getOrganizationCursor(second, "org-2", null), "page-2");
    assert.equal(getNextOrganizationCursor(second, "page-2", null), "page-3");
  });

  it("merges a server-rendered page without dropping previously loaded page mappings", () => {
    const loaded = createOrganizationPages([{ id: "org-1" }], null, "page-2");
    const directNavigation = addOrganizationPage(loaded, "page-3", {
      items: [{ id: "org-3" }],
      next_cursor: null,
    });

    assert.deepEqual(directNavigation.items.map(({ id }) => id), ["org-1", "org-3"]);
    assert.equal(getOrganizationCursor(directNavigation, "org-1", null), null);
    assert.equal(getOrganizationCursor(directNavigation, "org-3", null), "page-3");
  });
});

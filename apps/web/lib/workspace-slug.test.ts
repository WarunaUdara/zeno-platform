import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { isValidWorkspaceSlug, suggestedWorkspaceSlug } from "./workspace-slug";

describe("suggested workspace address", () => {
  it("normalizes accents, spacing, and punctuation", () => {
    assert.equal(suggestedWorkspaceSlug("Café & Arts Society!"), "cafe-arts-society");
  });

  it("trims long names to the API limit without a trailing separator", () => {
    assert.equal(suggestedWorkspaceSlug(`${"a".repeat(62)}-b`), "a".repeat(62));
  });

  it("uses an editable fallback when a name has no usable address characters", () => {
    assert.equal(suggestedWorkspaceSlug("舞台"), "workspace");
    assert.equal(suggestedWorkspaceSlug(" a "), "workspace");
  });

  it("accepts only addresses within the API's lowercase slug contract", () => {
    assert.equal(isValidWorkspaceSlug("campus-arts-2027"), true);
    assert.equal(isValidWorkspaceSlug("ca"), false);
    assert.equal(isValidWorkspaceSlug("Campus-Arts"), false);
    assert.equal(isValidWorkspaceSlug("-campus"), false);
    assert.equal(isValidWorkspaceSlug("a".repeat(63)), false);
  });
});

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { organizationChoiceHref, workspaceChooserHref } from "./workspace-chooser-links";

describe("workspace chooser navigation", () => {
  it("keeps a selected workspace attached to the page that contains it", () => {
    assert.equal(
      organizationChoiceHref("zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7N", "page-2_token"),
      "/organizer?organization=zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7N&org_cursor=page-2_token",
    );
  });

  it("paginates the chooser without carrying an unavailable organization ID", () => {
    assert.equal(
      workspaceChooserHref("next_page"),
      "/organizer?workspace_chooser=1&org_cursor=next_page",
    );
  });
});

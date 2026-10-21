import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { supportReference } from "./support-reference";

describe("support references", () => {
  it("accepts API request IDs from the response header or problem body", () => {
    const requestId = "req_01J9WZ6Q2CX7F2YV8AX0P4TD7K";
    assert.equal(supportReference(requestId), requestId);
    assert.equal(supportReference(null, requestId), requestId);
  });

  it("rejects malformed IDs and does not fall back from a present invalid header", () => {
    assert.equal(supportReference("<script>"), null);
    assert.equal(supportReference("req_too-short", "req_01J9WZ6Q2CX7F2YV8AX0P4TD7K"), null);
    assert.equal(supportReference(null, 123), null);
  });
});

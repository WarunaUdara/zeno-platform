import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  readReservationReleaseStatus,
  readReservationResponseBody,
} from "./reservation-response";

describe("reservation response parsing", () => {
  it("reads an RFC problem or reservation object", async () => {
    const response = new Response(JSON.stringify({ detail: "Try again", request_id: "req_01J9WZ6Q2CX7F2YV8AX0P4TD7K" }));
    assert.deepEqual(await readReservationResponseBody(response), {
      detail: "Try again",
      request_id: "req_01J9WZ6Q2CX7F2YV8AX0P4TD7K",
    });
  });

  it("returns null for an HTML gateway body instead of throwing", async () => {
    const response = new Response("<html>upstream unavailable</html>", {
      headers: { "Content-Type": "text/html" },
      status: 502,
    });
    assert.equal(await readReservationResponseBody(response), null);
  });

  it("rejects non-object JSON bodies", async () => {
    assert.equal(await readReservationResponseBody(new Response("[]")), null);
  });
});

describe("reservation release response parsing", () => {
  it("accepts the terminal statuses returned by the idempotent release endpoint", async () => {
    assert.equal(
      await readReservationReleaseStatus(new Response('{"status":"released"}')),
      "released",
    );
    assert.equal(
      await readReservationReleaseStatus(new Response('{"status":"expired"}')),
      "expired",
    );
  });

  it("treats malformed, unknown, and non-success responses as unconfirmed", async () => {
    assert.equal(
      await readReservationReleaseStatus(new Response("<html>gateway error</html>", { status: 502 })),
      null,
    );
    assert.equal(
      await readReservationReleaseStatus(new Response("{}")),
      null,
    );
    assert.equal(
      await readReservationReleaseStatus(new Response('{"status":"pending"}')),
      null,
    );
  });
});

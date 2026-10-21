import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { classifySessionLookup, classifyTokenLookup } from "./auth-result-state";

describe("Neon Auth server result handling", () => {
  it("distinguishes a missing session from an Auth-provider error", () => {
    assert.deepEqual(classifySessionLookup({ data: { user: null } }), { state: "signed-out" });
    assert.deepEqual(classifySessionLookup({ error: { status: 503 }, data: null }), { state: "unavailable" });
    assert.deepEqual(classifySessionLookup({ error: { status: 401 }, data: null }), { state: "signed-out" });
  });

  it("accepts only a successful session containing a user", () => {
    assert.deepEqual(classifySessionLookup({ data: { user: { id: "user-1" } } }), { state: "authenticated", user: { id: "user-1" } });
  });

  it("requires a non-empty token and a successful provider response", () => {
    assert.deepEqual(classifyTokenLookup({ data: { token: "signed.jwt" } }), { state: "ready", token: "signed.jwt" });
    assert.deepEqual(classifyTokenLookup({ data: { token: "" } }), { state: "unavailable" });
    assert.deepEqual(classifyTokenLookup({ error: { status: 503 }, data: null }), { state: "unavailable" });
    assert.deepEqual(classifyTokenLookup({ error: { status: 401 }, data: null }), { state: "signed-out" });
  });
});

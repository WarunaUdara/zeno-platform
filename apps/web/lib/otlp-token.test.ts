import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { createMetadataTokenProvider, type TokenFetch } from "./otlp-token";

const MINUTE = 60 * 1000;

function tokenResponse(accessToken: string, expiresInSeconds: number): Response {
  return new Response(
    JSON.stringify({ access_token: accessToken, expires_in: expiresInSeconds }),
    { status: 200, headers: { "content-type": "application/json" } },
  );
}

function recorder(): { fetchToken: TokenFetch; calls: string[]; headers: Array<Record<string, string>> } {
  const calls: string[] = [];
  const headers: Array<Record<string, string>> = [];
  const fetchToken: TokenFetch = (url, init) => {
    calls.push(url);
    headers.push((init.headers ?? {}) as Record<string, string>);
    return Promise.resolve(tokenResponse(`token-${calls.length}`, 3600));
  };
  return { fetchToken, calls, headers };
}

describe("metadata token provider", () => {
  it("asks the metadata server in the form it requires", async () => {
    const { fetchToken, calls, headers } = recorder();
    const accessToken = createMetadataTokenProvider(fetchToken);

    await accessToken();

    assert.equal(calls.length, 1);
    assert.match(calls[0], /\/computeMetadata\/v1\/instance\/service-accounts\/default\/token$/);
    // Without this header the metadata server refuses the request outright.
    assert.equal(headers[0]["Metadata-Flavor"], "Google");
  });

  it("reuses a live token so a span export is not a metadata round trip", async () => {
    const { fetchToken, calls } = recorder();
    const accessToken = createMetadataTokenProvider(fetchToken);

    assert.equal(await accessToken(), "token-1");
    assert.equal(await accessToken(), "token-1");
    assert.equal(calls.length, 1);
  });

  it("refreshes before the deadline rather than after it", async () => {
    const { fetchToken, calls } = recorder();
    let clock = 0;
    const accessToken = createMetadataTokenProvider(fetchToken, () => clock);

    await accessToken();

    // Half a minute inside the refresh margin, so still worth handing out.
    clock = 54 * MINUTE;
    assert.equal(await accessToken(), "token-1");
    assert.equal(calls.length, 1);

    // Now inside it: the token is still valid for four more minutes, which is not
    // long enough to be worth giving to a request.
    clock = 56 * MINUTE;
    assert.equal(await accessToken(), "token-2");
    assert.equal(calls.length, 2);
  });

  it("reports a refused request instead of returning an unusable token", async () => {
    const accessToken = createMetadataTokenProvider(() =>
      Promise.resolve(new Response("", { status: 403 })),
    );

    await assert.rejects(accessToken(), /403/);
  });
});

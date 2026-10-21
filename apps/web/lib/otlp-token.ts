const METADATA_HOST = process.env.GCE_METADATA_HOST ?? "metadata.google.internal";
const TOKEN_PATH = "/computeMetadata/v1/instance/service-accounts/default/token";

// Refresh this far ahead of the real deadline, so a token is never handed to a
// request that could outlive it.
const REFRESH_MARGIN_MS = 5 * 60 * 1000;

export type TokenFetch = (url: string, init: RequestInit) => Promise<Response>;

type TokenBody = {
  access_token: string;
  expires_in: number;
};

/**
 * A cached access token for the runtime service account.
 *
 * The exporter asks for headers on every batch, so without a cache each span export
 * would begin with a metadata round trip. The token is short-lived and minted by the
 * platform, which is the point: nothing is stored in the repository, in the image, or
 * in an environment variable, and there is no credential to rotate.
 *
 * `GCE_METADATA_HOST` is honoured because it is Google's own convention for pointing
 * the metadata client elsewhere, which is what makes this path testable off GCP.
 */
export function createMetadataTokenProvider(
  fetchToken: TokenFetch = fetch,
  now: () => number = Date.now,
) {
  let cached: { value: string; expiresAt: number } | undefined;

  return async function accessToken(): Promise<string> {
    const current = now();
    if (cached && cached.expiresAt - REFRESH_MARGIN_MS > current) {
      return cached.value;
    }

    const response = await fetchToken(`http://${METADATA_HOST}${TOKEN_PATH}`, {
      headers: { "Metadata-Flavor": "Google" },
    });
    if (!response.ok) {
      throw new Error(`metadata server refused the token request with ${response.status}`);
    }

    const body = (await response.json()) as TokenBody;
    cached = {
      value: body.access_token,
      expiresAt: current + body.expires_in * 1000,
    };
    return cached.value;
  };
}

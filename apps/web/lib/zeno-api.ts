import "server-only";
import { createZenoApiClient } from "@zeno/api-client";
import { resolveOutboundTraceparent } from "./trace-context";

const CLOUD_RUN_METADATA_URL =
  "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/identity";
const ID_TOKEN_CACHE_MS = 45 * 60 * 1000;

let cachedIdToken: { audience: string; value: string; expiresAt: number } | undefined;
let idTokenRequest: Promise<string> | undefined;

async function getCloudRunIdToken(audience: string): Promise<string> {
  if (
    cachedIdToken?.audience === audience &&
    cachedIdToken.expiresAt > Date.now()
  ) {
    return cachedIdToken.value;
  }

  if (!idTokenRequest) {
    idTokenRequest = (async () => {
      const metadataUrl = new URL(CLOUD_RUN_METADATA_URL);
      metadataUrl.searchParams.set("audience", audience);
      const response = await fetch(metadataUrl, {
        headers: { "Metadata-Flavor": "Google" },
        cache: "no-store",
        signal: AbortSignal.timeout(2_000),
      });
      if (!response.ok) {
        throw new Error(`Cloud Run identity token request failed (${response.status})`);
      }

      const token = (await response.text()).trim();
      if (!token || token.split(".").length !== 3) {
        throw new Error("Cloud Run returned an invalid identity token");
      }

      cachedIdToken = {
        audience,
        value: token,
        expiresAt: Date.now() + ID_TOKEN_CACHE_MS,
      };
      return token;
    })().finally(() => {
      idTokenRequest = undefined;
    });
  }

  return idTokenRequest;
}

type ApiCallLog = {
  event: string;
  method: string;
  path: string;
  traceparent: string;
  durationMs: number;
  status?: number;
  requestId?: string | null;
  detail?: string;
};

/**
 * Emit one structured line per failed upstream call. Only correlation fields are
 * logged: no headers, tokens, bodies, or query strings, because organizer and
 * attendee identifiers must not reach the log sink.
 */
function logApiCall(entry: ApiCallLog) {
  const line = JSON.stringify({
    timestamp: new Date().toISOString(),
    severity: entry.event === "zeno.api.upstream_error" ? "ERROR" : "WARNING",
    traceId: entry.traceparent.split("-")[1],
    parentSpanId: entry.traceparent.split("-")[2],
    ...entry,
  });
  if (entry.event === "zeno.api.upstream_error") console.error(line);
  else console.warn(line);
}

/**
 * Choose the trace context for an outgoing API call. A caller that already owns a
 * parent span (the reservation proxy) keeps it; anything else starts a new root
 * span so no web -> API hop is ever untraced.
 */
export function createServerApiClient(userToken?: string) {
  const configuredOrigin = process.env.ZENO_API_ORIGIN ?? "http://127.0.0.1:8080";
  const parsedOrigin = new URL(configuredOrigin);
  if (
    !["http:", "https:"].includes(parsedOrigin.protocol) ||
    parsedOrigin.username !== "" ||
    parsedOrigin.password !== "" ||
    parsedOrigin.pathname !== "/" ||
    parsedOrigin.search !== "" ||
    parsedOrigin.hash !== ""
  ) {
    throw new Error("ZENO_API_ORIGIN must be an HTTP(S) origin without a path or credentials");
  }

  const audience = process.env.ZENO_API_AUDIENCE;
  if (process.env.K_SERVICE && !audience) {
    throw new Error("ZENO_API_AUDIENCE must be configured when running on Cloud Run");
  }
  if (audience) {
    const parsedAudience = new URL(audience);
    if (
      parsedAudience.protocol !== "https:" ||
      parsedAudience.username !== "" ||
      parsedAudience.password !== "" ||
      parsedAudience.pathname !== "/" ||
      parsedAudience.search !== "" ||
      parsedAudience.hash !== ""
    ) {
      throw new Error("ZENO_API_AUDIENCE must be an HTTPS service origin without credentials or a path");
    }
  }

  return createZenoApiClient(parsedOrigin.origin, async (request) => {
    const headers = new Headers(request.headers);
    if (userToken) headers.set("Authorization", `Bearer ${userToken}`);
    if (audience) {
      headers.set("X-Serverless-Authorization", `Bearer ${await getCloudRunIdToken(audience)}`);
    }
    // Every web -> API hop must carry a trace context so a slow or failing API
    // span can be correlated with the web request that caused it. Callers that
    // already hold a parent span (the reservation proxy) keep it; everything else
    // starts a new root span here.
    const traceparent = resolveOutboundTraceparent(headers);
    headers.set("traceparent", traceparent);

    const url = new URL(request.url);
    const startedAt = performance.now();
    let response: Response;
    try {
      response = await fetch(new Request(request, { headers, cache: "no-store" }));
    } catch (error) {
      logApiCall({
        event: "zeno.api.upstream_unreachable",
        method: request.method,
        path: url.pathname,
        traceparent,
        durationMs: Math.round(performance.now() - startedAt),
        detail: error instanceof Error ? error.message : "unknown error",
      });
      throw error;
    }

    if (response.status >= 500) {
      logApiCall({
        event: "zeno.api.upstream_error",
        method: request.method,
        path: url.pathname,
        traceparent,
        status: response.status,
        durationMs: Math.round(performance.now() - startedAt),
        requestId: response.headers.get("X-Request-ID"),
      });
    }
    return response;
  });
}

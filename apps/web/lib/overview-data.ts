import "server-only";
import { unstable_cache } from "next/cache";
import type { components } from "@zeno/api-client";
import { createServerApiClient } from "./zeno-api";

// This app has not enabled Cache Components; keep these public reads in the
// server data cache with short revalidation windows instead of per-request fetches.
export type PublishedEventPage = components["schemas"]["PublishedEventPage"];
export type EventPreviewState =
  | { state: "available"; page: PublishedEventPage }
  | { state: "empty" }
  | { state: "unavailable" };
export const getOverviewEventPreview = unstable_cache(
  async (): Promise<EventPreviewState> => {
    try {
      const client = createServerApiClient();
      const result = await client.GET("/v1/events", {
        params: { query: { limit: 5 } },
        signal: AbortSignal.timeout(3_000),
      });
      const page = result.data as PublishedEventPage | undefined;
      if (!result.response.ok || !page) return { state: "unavailable" };
      return page.items.length > 0 ? { state: "available", page } : { state: "empty" };
    } catch {
      return { state: "unavailable" };
    }
  },
  ["zeno-overview-event-preview-v2"],
  { revalidate: 15 },
);

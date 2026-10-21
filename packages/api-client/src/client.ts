import createClient from "openapi-fetch";
import type { paths } from "./generated";

export function createZenoApiClient(
  baseUrl: string,
  fetchImplementation: (input: Request) => Promise<Response> = fetch,
) {
  return createClient<paths>({ baseUrl, fetch: fetchImplementation });
}

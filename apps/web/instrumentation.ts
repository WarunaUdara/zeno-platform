/**
 * Next.js runs `register` before any application code in a new server process,
 * which is the only point early enough to install a tracer provider and have it be
 * the parent of the first request span.
 *
 * Both gates are deliberate. `NEXT_RUNTIME` keeps the Node SDK out of the edge
 * bundle, where it cannot run. `OTEL_EXPORTER_OTLP_ENDPOINT` makes export opt-in:
 * unset, no provider is installed at all, so local runs, tests, and CI behave
 * exactly as they did before tracing existed.
 */
export async function register(): Promise<void> {
  if (process.env.NEXT_RUNTIME !== "nodejs") {
    return;
  }
  if (!process.env.OTEL_EXPORTER_OTLP_ENDPOINT?.trim()) {
    return;
  }

  await import("./instrumentation.node");
}

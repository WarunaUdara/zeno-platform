import { DiagConsoleLogger, DiagLogLevel, diag } from "@opentelemetry/api";
import { OTLPTraceExporter } from "@opentelemetry/exporter-trace-otlp-http";
import { resourceFromAttributes } from "@opentelemetry/resources";
import { NodeSDK } from "@opentelemetry/sdk-node";
import { BatchSpanProcessor } from "@opentelemetry/sdk-trace-node";
import type { SpanExporter } from "@opentelemetry/sdk-trace-node";
import { ATTR_SERVICE_NAME } from "@opentelemetry/semantic-conventions";
import { createMetadataTokenProvider } from "./lib/otlp-token";

const LEVELS: Record<string, DiagLogLevel> = {
  none: DiagLogLevel.NONE,
  error: DiagLogLevel.ERROR,
  warn: DiagLogLevel.WARN,
  info: DiagLogLevel.INFO,
  debug: DiagLogLevel.DEBUG,
  verbose: DiagLogLevel.VERBOSE,
};

/**
 * Install a diagnostic logger before the SDK is built, not after.
 *
 * `NodeSDK.start()` is itself what reacts to `OTEL_LOG_LEVEL`, so until it returns
 * there is no logger at all: a failure inside `start()`, or inside the exporter
 * `start()` builds, reports to nothing. That is the worst shape a failure can take,
 * because the process looks healthy, serves traffic, and exports no spans.
 *
 * The default is `info` rather than `error` so the one-line posture message below is
 * visible without anyone having to know it exists. The SDK dumps a span per batch at
 * `debug`, which is why the level is set here and left alone.
 */
function installDiagnostics(): void {
  const requested = process.env.OTEL_LOG_LEVEL?.trim().toLowerCase();
  const level = (requested && LEVELS[requested]) || DiagLogLevel.INFO;
  diag.setLogger(new DiagConsoleLogger(), level);
}

/**
 * Report an export that the collector or Cloud Trace rejected.
 *
 * The Node transport resolves a non-2xx response to `{ status: 'failure' }` and logs
 * nothing, so from inside the process a rejected batch and a delivered one are
 * indistinguishable: both are silent, and the only difference is that the spans never
 * appear. Logging the failure is what makes that observable.
 */
class ReportingExporter implements SpanExporter {
  constructor(private readonly inner: SpanExporter) {}

  export: SpanExporter["export"] = (spans, done) => {
    this.inner.export(spans, (result) => {
      if ("error" in result) {
        diag.error(
          `otlp export failed, ${spans.length} span(s) not recorded`,
          result.error,
        );
      }
      done(result);
    });
  };

  shutdown(): Promise<void> {
    return this.inner.shutdown();
  }

  forceFlush(): Promise<void> {
    return this.inner.forceFlush?.() ?? Promise.resolve();
  }
}

const accessToken = createMetadataTokenProvider();

/**
 * Cloud Run names the service in `K_SERVICE`, so each workload is separable in
 * Cloud Trace without extra configuration. This mirrors `telemetry.rs` exactly, so
 * a trace that crosses tiers is labelled consistently at both ends.
 */
function serviceName(): string {
  return process.env.OTEL_SERVICE_NAME ?? process.env.K_SERVICE ?? "zeno-web";
}

/**
 * Cloud Trace files a span under a project by this attribute. Cloud Run sets the
 * variable; without it, as in a local test, the attribute is simply absent.
 */
function resource() {
  const attributes: Record<string, string> = { [ATTR_SERVICE_NAME]: serviceName() };
  const projectId = process.env.GOOGLE_CLOUD_PROJECT?.trim();
  if (projectId) {
    attributes["gcp.project_id"] = projectId;
  }
  return resourceFromAttributes(attributes);
}

/**
 * The exporter calls this on every batch, and Cloud Trace rejects a span without a
 * bearer token. It must not throw: the SDK's contract is that a headers factory
 * returns a map, so a failure here is reported and then surfaces as a rejected
 * export rather than as an exception inside the exporter's own loop.
 */
async function authHeaders(): Promise<Record<string, string>> {
  try {
    return { authorization: `Bearer ${await accessToken()}` };
  } catch (error) {
    diag.error("otlp token unavailable, exporting without credentials", error);
    return {};
  }
}

installDiagnostics();

// No `url` is passed on purpose. The SDK reads `OTEL_EXPORTER_OTLP_ENDPOINT` and
// appends the signal path; an endpoint supplied here would be used verbatim and
// Cloud Trace would reject the bare host. That is the same reasoning as the API's
// exporter, and it keeps both tiers resolving the endpoint by one rule.
const exporter = new OTLPTraceExporter({ headers: authHeaders });

try {
  new NodeSDK({
    resource: resource(),
    spanProcessors: [new BatchSpanProcessor(new ReportingExporter(exporter))],
  }).start();
  // Which identity this process exports as, and where to. A trace list is hard to
  // reason about when the label on it is a guess, and `service.name` here is derived
  // from a platform variable rather than written down anywhere.
  diag.info(
    `otlp span export enabled: service=${serviceName()} ` +
      `endpoint=${process.env.OTEL_EXPORTER_OTLP_ENDPOINT} ` +
      `project=${process.env.GOOGLE_CLOUD_PROJECT ?? "(unset)"}`,
  );
} catch (error) {
  // Tracing fails open, unlike the ticket credential key, which fails closed. Losing
  // spans costs visibility; refusing to serve costs the event.
  diag.error("opentelemetry did not start, spans will not be exported", error);
}

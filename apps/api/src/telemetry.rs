//! OTLP span export to Cloud Trace.
//!
//! Export is off unless `OTEL_EXPORTER_OTLP_ENDPOINT` is set. Local runs, the
//! test suite, and CI therefore behave exactly as they did before this module
//! existed: no exporter, no background thread, no network call, no overhead.
//!
//! The exporter authenticates with the instance's service account. A Cloud Run
//! container reaches the metadata server without configuration, and the token it
//! hands back expires after about an hour, so the exporter cannot be given a
//! static `Authorization` header at build time. Instead this module supplies its
//! own HTTP client, which fetches a token, caches it, and refreshes it before it
//! lapses.

use std::{
    env,
    sync::RwLock,
    time::{Duration, Instant},
};

use opentelemetry::{
    Context, KeyValue,
    trace::{
        SpanContext, SpanId, TraceContextExt, TraceFlags, TraceId, TraceState, TracerProvider as _,
    },
};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_otlp::{SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{
    Resource,
    trace::{Sampler, SdkTracerProvider},
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// Refresh this far ahead of expiry so an export never races the clock.
const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(120);

/// A token request that hangs must not hold an export batch open.
const TOKEN_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

/// A single export attempt must finish well inside Cloud Run's termination grace
/// period, so the final flush on shutdown completes instead of being killed.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(5);

/// Google's client libraries read this so a local metadata emulator can stand in
/// for the real server. Honouring it is what makes this exporter testable
/// anywhere other than Google Cloud.
fn metadata_token_url() -> String {
    let host =
        env::var("GCE_METADATA_HOST").unwrap_or_else(|_| "metadata.google.internal".to_owned());
    format!("http://{host}/computeMetadata/v1/instance/service-accounts/default/token")
}

/// An export response is an acknowledgement, so a large one is a fault. The
/// limit matches the one `opentelemetry-http` applies to its own clients.
const MAX_RESPONSE_BODY_BYTES: usize = 4 * 1024 * 1024;

/// Holds the tracer provider so spans can be flushed when the process stops.
pub struct Telemetry {
    provider: Option<SdkTracerProvider>,
}

impl Drop for Telemetry {
    fn drop(&mut self) {
        let Some(provider) = self.provider.take() else {
            return;
        };
        if let Err(error) = provider.shutdown() {
            // Stderr rather than tracing: the subscriber may already be gone.
            eprintln!("otlp exporter shutdown failed: {error}");
        }
    }
}

/// Installs the tracing subscriber, with OTLP export when it is configured.
///
/// The returned guard must live for the whole process. Dropping it flushes
/// whatever the batch processor is still holding.
#[must_use]
pub fn init() -> Telemetry {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let provider = build_provider();
    let otel_layer = provider.as_ref().map(|provider| {
        tracing_opentelemetry::layer().with_tracer(provider.tracer(service_name()))
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().json().with_target(false))
        .with(otel_layer)
        .init();

    Telemetry { provider }
}

/// Builds a remote parent from the W3C identifiers the API already parsed.
///
/// The request path generates or parses a `traceparent` before any span exists,
/// and the JSON logs report those identifiers. Adopting them as the span's parent
/// is what makes a log line and a trace in Cloud Trace the same request; without
/// it the two would carry unrelated trace ids and neither could be used to find
/// the other.
pub fn parent_context(trace_id: &str, span_id: &str) -> Option<Context> {
    let trace_id = TraceId::from_hex(trace_id).ok()?;
    let span_id = SpanId::from_hex(span_id).ok()?;
    let span_context = SpanContext::new(
        trace_id,
        span_id,
        TraceFlags::SAMPLED,
        true,
        TraceState::default(),
    );
    span_context
        .is_valid()
        .then(|| Context::new().with_remote_span_context(span_context))
}

fn build_provider() -> Option<SdkTracerProvider> {
    export_endpoint()?;
    let exporter = match SpanExporter::builder()
        .with_http()
        .with_timeout(EXPORT_TIMEOUT)
        .with_http_client(AuthedClient::new())
        .build()
    {
        Ok(exporter) => exporter,
        Err(error) => {
            // Export is a diagnostic aid. A misconfigured exporter must not stop
            // the API from serving.
            eprintln!("otlp exporter disabled: {error}");
            return None;
        }
    };

    Some(
        SdkTracerProvider::builder()
            .with_resource(resource())
            .with_sampler(sampler())
            .with_batch_exporter(exporter)
            .build(),
    )
}

/// Export is opt-in: an absent endpoint means the process runs untraced.
///
/// This only decides whether to export. The endpoint itself is left to the SDK,
/// which reads the same variable and appends the signal path (`/v1/traces`) to
/// it. Handing the value to `with_endpoint` instead would suppress that append:
/// a programmatic endpoint is used verbatim, so the exporter would post to the
/// bare host and Cloud Trace would reject every request.
fn export_endpoint() -> Option<String> {
    let endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok()?;
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return None;
    }
    Some(endpoint.to_owned())
}

fn resource() -> Resource {
    let mut builder = Resource::builder().with_service_name(service_name());
    // Cloud Trace files a span under a project by this attribute. Cloud Run sets
    // the environment variable; without it (local testing) the attribute is
    // simply absent.
    if let Ok(project_id) = env::var("GOOGLE_CLOUD_PROJECT")
        && !project_id.trim().is_empty()
    {
        builder = builder.with_attribute(KeyValue::new("gcp.project_id", project_id));
    }
    builder.build()
}

/// Cloud Run names the service in `K_SERVICE`, so each workload is separable in
/// Cloud Trace without extra configuration.
fn service_name() -> String {
    env::var("OTEL_SERVICE_NAME")
        .or_else(|_| env::var("K_SERVICE"))
        .unwrap_or_else(|_| "zeno-api".to_owned())
}

/// Sampling inherits the upstream decision and falls back to a trace-id ratio when
/// there is none.
///
/// The web tier now exports spans, so the root of a browser request is sampled once,
/// there, and the API follows that decision. Deciding independently is what produces
/// half-traces: a request whose web span exists and whose API span does not, which is
/// worse than no trace because it looks complete.
fn sampler() -> Sampler {
    let ratio = env::var("ZENO_OTLP_SAMPLE_RATIO").ok();
    Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(parse_sample_ratio(
        ratio.as_deref(),
    ))))
}

/// A ratio that is not a ratio is a configuration mistake, not a reason to stop
/// exporting, so it falls back to exporting everything.
fn parse_sample_ratio(value: Option<&str>) -> f64 {
    value
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|ratio| (0.0..=1.0).contains(ratio))
        .unwrap_or(1.0)
}

#[derive(Debug)]
struct AuthedClient {
    inner: reqwest::blocking::Client,
    tokens: TokenCache,
}

impl AuthedClient {
    fn new() -> Self {
        Self {
            inner: reqwest::blocking::Client::new(),
            tokens: TokenCache::new(),
        }
    }
}

/// The SDK runs batch exports on its own threads, which is why the blocking
/// client is the documented pairing for `BatchSpanProcessor`. Blocking here
/// stalls that thread and never the request path.
///
/// The request is sent by hand rather than through `opentelemetry-http`'s own
/// `reqwest` implementation: that crate builds against reqwest 0.13 while this
/// workspace pins 0.12, so its implementation is for a different type. Sending
/// it here keeps one reqwest in the binary instead of two.
#[async_trait::async_trait]
impl HttpClient for AuthedClient {
    async fn send_bytes(&self, mut request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        use std::io::Read;

        let header = format!("Bearer {}", self.tokens.bearer()?)
            .parse()
            .map_err(|_| "service account token is not a valid header value")?;
        request.headers_mut().insert("authorization", header);

        let (parts, body) = request.into_parts();
        let response = self
            .inner
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(body.to_vec())
            .send()?;

        let status = response.status();
        let headers = response.headers().clone();
        let mut body_bytes = Vec::new();
        response
            .take(MAX_RESPONSE_BODY_BYTES as u64 + 1)
            .read_to_end(&mut body_bytes)?;
        if body_bytes.len() > MAX_RESPONSE_BODY_BYTES {
            return Err("otlp response body exceeded the limit".into());
        }

        let mut http_response = Response::builder()
            .status(status)
            .body(Bytes::from(body_bytes))?;
        *http_response.headers_mut() = headers;
        Ok(http_response)
    }
}

#[derive(Debug)]
struct TokenCache {
    client: reqwest::blocking::Client,
    cached: RwLock<Option<CachedToken>>,
}

#[derive(Debug)]
struct CachedToken {
    token: String,
    expires_at: Instant,
}

impl TokenCache {
    fn new() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(TOKEN_REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
            cached: RwLock::new(None),
        }
    }

    fn bearer(&self) -> Result<String, HttpError> {
        let refresh_before = Instant::now() + TOKEN_REFRESH_MARGIN;
        if let Some(cached) = self
            .cached
            .read()
            .map_err(|_| "service account token cache is poisoned")?
            .as_ref()
            && cached.expires_at > refresh_before
        {
            return Ok(cached.token.clone());
        }

        let fresh = self.fetch()?;
        let token = fresh.token.clone();
        *self
            .cached
            .write()
            .map_err(|_| "service account token cache is poisoned")? = Some(fresh);
        Ok(token)
    }

    fn fetch(&self) -> Result<CachedToken, HttpError> {
        let response = self
            .client
            .get(metadata_token_url())
            .header("Metadata-Flavor", "Google")
            .send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("metadata server returned {status}").into());
        }
        let body: MetadataToken = serde_json::from_str(&response.text()?)?;
        Ok(CachedToken {
            token: body.access_token,
            expires_at: Instant::now() + Duration::from_secs(body.expires_in),
        })
    }
}

#[derive(serde::Deserialize)]
struct MetadataToken {
    access_token: String,
    expires_in: u64,
}

#[cfg(test)]
mod tests {
    use opentelemetry::trace::TraceContextExt;

    use super::{parent_context, parse_sample_ratio};

    #[test]
    fn a_parsed_traceparent_becomes_a_usable_parent() {
        let context = parent_context("4bf92f3577b34da6a3ce929d0e0e4736", "00f067aa0ba902b7")
            .expect("valid identifiers should build a context");
        let span = context.span().span_context().clone();
        assert!(span.is_valid());
        assert!(span.is_remote());
        assert_eq!(
            span.trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
    }

    #[test]
    fn malformed_or_empty_identifiers_do_not_become_a_parent() {
        assert!(parent_context("", "").is_none());
        assert!(parent_context("not-hex", "00f067aa0ba902b7").is_none());
        assert!(parent_context("4bf92f3577b34da6a3ce929d0e0e4736", "").is_none());
        // All-zero identifiers are explicitly invalid in W3C trace context.
        assert!(parent_context("00000000000000000000000000000000", "0000000000000000").is_none());
    }

    #[test]
    fn an_unusable_sample_ratio_exports_everything_rather_than_nothing() {
        for value in [
            None,
            Some("-0.1"),
            Some("1.1"),
            Some("not-a-number"),
            Some(""),
        ] {
            assert_eq!(parse_sample_ratio(value), 1.0, "{value:?}");
        }
        assert_eq!(parse_sample_ratio(Some("0")), 0.0);
        assert_eq!(parse_sample_ratio(Some("0.25")), 0.25);
        assert_eq!(parse_sample_ratio(Some(" 1 ")), 1.0);
    }
}

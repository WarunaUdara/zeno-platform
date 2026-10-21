use std::{sync::Arc, time::Instant};

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use prometheus::{
    Encoder, Gauge, GaugeVec, HistogramVec, IntCounterVec, IntGauge, Opts, Registry, TextEncoder,
};
use sqlx::PgPool;

#[derive(Clone)]
pub(crate) struct ApiMetrics {
    registry: Arc<Registry>,
    requests: IntCounterVec,
    request_duration: HistogramVec,
    in_flight: IntGauge,
    database_connections: IntGauge,
    database_idle_connections: IntGauge,
    worker_last_batch_timestamp: GaugeVec,
    worker_last_batch_succeeded: GaugeVec,
    worker_last_batch_duration: GaugeVec,
    worker_consecutive_failures: GaugeVec,
    outbox_due_jobs: IntGauge,
    outbox_oldest_due_job_age: Gauge,
}

impl ApiMetrics {
    pub(crate) fn new() -> Self {
        let registry = Registry::new();
        let requests = IntCounterVec::new(
            Opts::new(
                "zeno_http_requests_total",
                "HTTP requests served by the API.",
            ),
            &["method", "route", "status_code"],
        )
        .expect("valid request counter definition");
        let request_duration = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "zeno_http_request_duration_seconds",
                "HTTP request duration in seconds.",
            )
            .buckets(vec![
                0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
            ]),
            &["method", "route"],
        )
        .expect("valid request duration definition");
        let in_flight = IntGauge::new(
            "zeno_http_requests_in_flight",
            "HTTP requests currently being handled.",
        )
        .expect("valid in-flight gauge definition");
        let database_connections = IntGauge::new(
            "zeno_database_pool_connections",
            "Connections currently managed by the API PostgreSQL pool.",
        )
        .expect("valid database connection gauge definition");
        let database_idle_connections = IntGauge::new(
            "zeno_database_pool_idle_connections",
            "Idle connections currently available in the API PostgreSQL pool.",
        )
        .expect("valid idle database connection gauge definition");

        let worker_last_batch_timestamp = GaugeVec::new(
            Opts::new(
                "zeno_worker_last_batch_timestamp_seconds",
                "Unix timestamp of the worker instance's latest completed batch.",
            ),
            &["worker_name", "instance_id"],
        )
        .expect("valid worker timestamp definition");
        let worker_last_batch_succeeded = GaugeVec::new(
            Opts::new(
                "zeno_worker_last_batch_succeeded",
                "Whether the worker instance's latest batch succeeded (1) or failed (0).",
            ),
            &["worker_name", "instance_id"],
        )
        .expect("valid worker result definition");
        let worker_last_batch_duration = GaugeVec::new(
            Opts::new(
                "zeno_worker_last_batch_duration_seconds",
                "How long the worker instance's latest batch took to complete.",
            ),
            &["worker_name", "instance_id"],
        )
        .expect("valid worker duration definition");
        let worker_consecutive_failures = GaugeVec::new(
            Opts::new(
                "zeno_worker_consecutive_failures",
                "Consecutive failed batches for a worker instance.",
            ),
            &["worker_name", "instance_id"],
        )
        .expect("valid worker failure count definition");
        let outbox_due_jobs = IntGauge::new(
            "zeno_outbox_due_jobs",
            "Pending outbox jobs whose scheduled time has arrived.",
        )
        .expect("valid outbox due job gauge definition");
        let outbox_oldest_due_job_age = Gauge::new(
            "zeno_outbox_oldest_due_job_age_seconds",
            "Age of the oldest pending outbox job that is due for processing.",
        )
        .expect("valid outbox job age gauge definition");

        registry
            .register(Box::new(requests.clone()))
            .expect("unique request counter name");
        registry
            .register(Box::new(request_duration.clone()))
            .expect("unique request duration name");
        registry
            .register(Box::new(in_flight.clone()))
            .expect("unique in-flight gauge name");
        registry
            .register(Box::new(database_connections.clone()))
            .expect("unique database connection gauge name");
        registry
            .register(Box::new(database_idle_connections.clone()))
            .expect("unique database idle gauge name");
        registry
            .register(Box::new(worker_last_batch_timestamp.clone()))
            .expect("unique worker timestamp name");
        registry
            .register(Box::new(worker_last_batch_succeeded.clone()))
            .expect("unique worker result name");
        registry
            .register(Box::new(worker_last_batch_duration.clone()))
            .expect("unique worker duration name");
        registry
            .register(Box::new(worker_consecutive_failures.clone()))
            .expect("unique worker failure count name");
        registry
            .register(Box::new(outbox_due_jobs.clone()))
            .expect("unique outbox due job name");
        registry
            .register(Box::new(outbox_oldest_due_job_age.clone()))
            .expect("unique outbox age name");

        Self {
            registry: Arc::new(registry),
            requests,
            request_duration,
            in_flight,
            database_connections,
            database_idle_connections,
            worker_last_batch_timestamp,
            worker_last_batch_succeeded,
            worker_last_batch_duration,
            worker_consecutive_failures,
            outbox_due_jobs,
            outbox_oldest_due_job_age,
        }
    }

    fn refresh_pool_metrics(&self, pool: &PgPool) {
        self.database_connections.set(i64::from(pool.size()));
        self.database_idle_connections
            .set(i64::try_from(pool.num_idle()).unwrap_or(i64::MAX));
    }

    async fn refresh_worker_metrics(&self, pool: &PgPool) -> Result<(), sqlx::Error> {
        let rows = sqlx::query_as::<_, WorkerHeartbeatRow>(
            "SELECT worker_name, instance_id, last_batch_at, last_batch_succeeded, \
                    consecutive_failures, last_batch_duration_ms \
             FROM worker_heartbeats \
             WHERE last_batch_at >= clock_timestamp() - interval '24 hours'",
        )
        .fetch_all(pool)
        .await?;

        self.worker_last_batch_timestamp.reset();
        self.worker_last_batch_succeeded.reset();
        self.worker_last_batch_duration.reset();
        self.worker_consecutive_failures.reset();
        for row in rows {
            let labels = [row.worker_name.as_str(), row.instance_id.as_str()];
            self.worker_last_batch_timestamp
                .with_label_values(&labels)
                .set(row.last_batch_at.timestamp() as f64);
            self.worker_last_batch_succeeded
                .with_label_values(&labels)
                .set(if row.last_batch_succeeded { 1.0 } else { 0.0 });
            // A row written before the duration column existed reports nothing
            // rather than zero, so it cannot look like an instant batch.
            if let Some(duration_ms) = row.last_batch_duration_ms {
                self.worker_last_batch_duration
                    .with_label_values(&labels)
                    .set(f64::from(duration_ms) / 1000.0);
            }
            self.worker_consecutive_failures
                .with_label_values(&labels)
                .set(f64::from(row.consecutive_failures));
        }
        let (due_jobs, oldest_due_age): (i64, f64) = sqlx::query_as(
            "SELECT count(*)::bigint, \
                    COALESCE(EXTRACT(EPOCH FROM clock_timestamp() - min(available_at)), 0)::double precision \
             FROM outbox_jobs \
             WHERE status = 'pending' AND available_at <= clock_timestamp()",
        )
        .fetch_one(pool)
        .await?;
        self.outbox_due_jobs.set(due_jobs);
        self.outbox_oldest_due_job_age.set(oldest_due_age);
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct WorkerHeartbeatRow {
    worker_name: String,
    instance_id: String,
    last_batch_at: DateTime<Utc>,
    last_batch_succeeded: bool,
    consecutive_failures: i32,
    last_batch_duration_ms: Option<i32>,
}

struct InFlightGuard(IntGauge);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.dec();
    }
}

pub(crate) async fn record_request(
    State(metrics): State<ApiMetrics>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.uri().path() == "/metrics" {
        return next.run(request).await;
    }

    let method = match request.method().as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "OTHER",
    };
    let route = route_label(method, request.uri().path());
    metrics.in_flight.inc();
    let _in_flight_guard = InFlightGuard(metrics.in_flight.clone());
    let started = Instant::now();
    let response = next.run(request).await;
    let status = response.status().as_u16().to_string();
    metrics
        .requests
        .with_label_values(&[method, route, &status])
        .inc();
    metrics
        .request_duration
        .with_label_values(&[method, route])
        .observe(started.elapsed().as_secs_f64());
    response
}

pub(crate) fn route_label(method: &str, path: &str) -> &'static str {
    match (method, path) {
        ("GET", "/health/live") => return "/health/live",
        ("GET", "/health/ready") => return "/health/ready",
        ("GET", "/metrics") => return "/metrics",
        ("GET", "/v1/auth/me") => return "/v1/auth/me",
        ("GET", "/v1/organizations") => return "/v1/organizations",
        ("POST", "/v1/organizations") => return "/v1/organizations",
        _ => {}
    }
    if let Some(organization_path) = path.strip_prefix("/v1/organizations/") {
        let mut segments = organization_path.split('/');
        let organization_id = segments.next().unwrap_or_default();
        let resource = segments.next();
        let event_id = segments.next();
        let action = segments.next();
        let extra = segments.next();
        return match (
            method,
            organization_id.is_empty(),
            resource,
            event_id,
            action,
            extra,
        ) {
            ("GET", false, Some("events"), None, None, None) => {
                "/v1/organizations/{organization_id}/events"
            }
            ("POST", false, Some("events"), None, None, None) => {
                "/v1/organizations/{organization_id}/events"
            }
            ("GET", false, Some("events"), Some(event_id), None, None) if !event_id.is_empty() => {
                "/v1/organizations/{organization_id}/events/{event_id}"
            }
            ("PUT", false, Some("events"), Some(event_id), None, None) if !event_id.is_empty() => {
                "/v1/organizations/{organization_id}/events/{event_id}"
            }
            ("GET", false, Some("events"), Some(event_id), Some("ticket-types"), None)
                if !event_id.is_empty() =>
            {
                "/v1/organizations/{organization_id}/events/{event_id}/ticket-types"
            }
            ("POST", false, Some("events"), Some(event_id), Some("ticket-types"), None)
                if !event_id.is_empty() =>
            {
                "/v1/organizations/{organization_id}/events/{event_id}/ticket-types"
            }
            (
                "PUT",
                false,
                Some("events"),
                Some(event_id),
                Some("ticket-types"),
                Some(ticket_type_id),
            ) if !event_id.is_empty() && !ticket_type_id.is_empty() => {
                "/v1/organizations/{organization_id}/events/{event_id}/ticket-types/{ticket_type_id}"
            }
            ("POST", false, Some("events"), Some(event_id), Some("publish"), None)
                if !event_id.is_empty() =>
            {
                "/v1/organizations/{organization_id}/events/{event_id}/publish"
            }
            ("POST", false, Some("events"), Some(event_id), Some("cover-uploads"), None)
                if !event_id.is_empty() =>
            {
                "/v1/organizations/{organization_id}/events/{event_id}/cover-uploads"
            }
            ("PUT", false, Some("events"), Some(event_id), Some("cover-image"), None)
                if !event_id.is_empty() =>
            {
                "/v1/organizations/{organization_id}/events/{event_id}/cover-image"
            }
            ("PUT", false, Some("events"), Some(event_id), Some("registration-form"), None)
                if !event_id.is_empty() =>
            {
                "/v1/organizations/{organization_id}/events/{event_id}/registration-form"
            }
            ("POST", false, Some("events"), Some(event_id), Some("check-ins"), None)
                if !event_id.is_empty() =>
            {
                "/v1/organizations/{organization_id}/events/{event_id}/check-ins"
            }
            ("GET", false, Some("events"), Some(event_id), Some("attendance"), None)
                if !event_id.is_empty() =>
            {
                "/v1/organizations/{organization_id}/events/{event_id}/attendance"
            }
            _ => "unmatched",
        };
    }
    if method == "GET" && path == "/v1/events" {
        return "/v1/events";
    }
    let Some(resource_path) = path.strip_prefix("/v1/events/") else {
        return "unmatched";
    };
    let mut segments = resource_path.split('/');
    let event_id = segments.next().unwrap_or_default();
    let second = segments.next();
    let third = segments.next();
    let fourth = segments.next();
    if segments.next().is_some() {
        return "unmatched";
    }

    match (method, event_id.is_empty(), second, third, fourth) {
        ("GET", false, None, None, None) => "/v1/events/{event_id}",
        ("GET", false, Some("registration-form"), None, None) => {
            "/v1/events/{event_id}/registration-form"
        }
        ("GET", false, Some("ticket-types"), None, None) => "/v1/events/{event_id}/ticket-types",
        ("POST", false, Some("ticket-types"), Some(ticket_type_id), Some("reservations"))
            if !ticket_type_id.is_empty() =>
        {
            "/v1/events/{event_id}/ticket-types/{ticket_type_id}/reservations"
        }
        ("DELETE", false, Some("reservations"), Some(reservation_id), None)
            if !reservation_id.is_empty() =>
        {
            "/v1/events/{event_id}/reservations/{reservation_id}"
        }
        ("POST", false, Some("reservations"), Some(reservation_id), Some("confirm"))
            if !reservation_id.is_empty() =>
        {
            "/v1/events/{event_id}/reservations/{reservation_id}/confirm"
        }
        _ => "unmatched",
    }
}

pub(crate) async fn render_metrics(State(state): State<crate::app::AppState>) -> Response {
    if let Err(error) = state.metrics.refresh_worker_metrics(&state.pool).await {
        // A scrape that returns 503 with no explanation is invisible in the logs
        // and reads as a dead target instead of a failing worker-heartbeat query.
        crate::db_error::record(error, "refresh_worker_metrics");
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    state.metrics.refresh_pool_metrics(&state.pool);
    let encoder = TextEncoder::new();
    let mut body = Vec::new();
    if encoder
        .encode(&state.metrics.registry.gather(), &mut body)
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, encoder.format_type())],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::route_label;

    #[test]
    fn request_labels_use_route_templates_and_bound_unmatched_paths() {
        assert_eq!(route_label("GET", "/v1/events"), "/v1/events");
        assert_eq!(route_label("GET", "/v1/auth/me"), "/v1/auth/me");
        assert_eq!(route_label("GET", "/v1/organizations"), "/v1/organizations");
        assert_eq!(
            route_label("POST", "/v1/organizations"),
            "/v1/organizations"
        );
        assert_eq!(
            route_label(
                "POST",
                "/v1/organizations/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7N/events/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7K/ticket-types"
            ),
            "/v1/organizations/{organization_id}/events/{event_id}/ticket-types"
        );
        assert_eq!(
            route_label(
                "PUT",
                "/v1/organizations/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7N/events/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7K/ticket-types/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7L"
            ),
            "/v1/organizations/{organization_id}/events/{event_id}/ticket-types/{ticket_type_id}"
        );
        assert_eq!(
            route_label("GET", "/v1/events/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7K"),
            "/v1/events/{event_id}"
        );
        assert_eq!(
            route_label(
                "POST",
                "/v1/events/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7K/ticket-types/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7L/reservations"
            ),
            "/v1/events/{event_id}/ticket-types/{ticket_type_id}/reservations"
        );
        assert_eq!(
            route_label("GET", "/v1/events/secret/user@example.com"),
            "unmatched"
        );
    }

    /// Every route the router mounts must resolve to its own template. An
    /// unlabelled route collapses into "unmatched" and hides real traffic from
    /// the dashboards and error-budget alerts.
    #[test]
    fn every_mounted_organizer_and_reservation_route_has_a_template() {
        let organizer_event = "/v1/organizations/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7N/events/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7K";
        assert_eq!(
            route_label("GET", organizer_event),
            "/v1/organizations/{organization_id}/events/{event_id}"
        );
        assert_eq!(
            route_label("PUT", organizer_event),
            "/v1/organizations/{organization_id}/events/{event_id}"
        );
        assert_eq!(
            route_label("POST", &format!("{organizer_event}/cover-uploads")),
            "/v1/organizations/{organization_id}/events/{event_id}/cover-uploads"
        );
        assert_eq!(
            route_label("PUT", &format!("{organizer_event}/cover-image")),
            "/v1/organizations/{organization_id}/events/{event_id}/cover-image"
        );
        assert_eq!(
            route_label(
                "DELETE",
                "/v1/events/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7K/reservations/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7L"
            ),
            "/v1/events/{event_id}/reservations/{reservation_id}"
        );
    }

    /// Ordering, forms and door traffic are the paths that carry revenue and
    /// admission, so they must be visible individually rather than lumped into
    /// "unmatched" where no alert can separate them.
    #[test]
    fn ordering_form_and_door_routes_have_templates() {
        let event = "/v1/events/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7K";
        let reservation = "zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7L";
        assert_eq!(
            route_label("GET", &format!("{event}/registration-form")),
            "/v1/events/{event_id}/registration-form"
        );
        assert_eq!(
            route_label(
                "POST",
                &format!("{event}/reservations/{reservation}/confirm")
            ),
            "/v1/events/{event_id}/reservations/{reservation_id}/confirm"
        );

        let organizer_event = "/v1/organizations/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7N/events/zeno_01J9WZ6Q2CX7F2YV8AX0P4TD7K";
        assert_eq!(
            route_label("PUT", &format!("{organizer_event}/registration-form")),
            "/v1/organizations/{organization_id}/events/{event_id}/registration-form"
        );
        assert_eq!(
            route_label("POST", &format!("{organizer_event}/check-ins")),
            "/v1/organizations/{organization_id}/events/{event_id}/check-ins"
        );
        assert_eq!(
            route_label("GET", &format!("{organizer_event}/attendance")),
            "/v1/organizations/{organization_id}/events/{event_id}/attendance"
        );
    }
}

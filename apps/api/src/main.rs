use std::{env, error::Error, net::IpAddr, net::SocketAddr, str::FromStr, time::Duration};

use sqlx::{PgPool, postgres::PgConnectOptions, postgres::PgPoolOptions};
use tokio::net::TcpListener;
use zeno_api::{app, auth::NeonJwtVerifier};

/// Builds telemetry before the async runtime exists.
///
/// The OTLP exporter owns a blocking HTTP client, and reqwest's blocking client
/// panics if one is created or dropped from inside an async context -- which is
/// exactly where a `#[tokio::main]` body runs. Dropping the guard here, after
/// `block_on` returns, also puts the final span flush outside the runtime.
fn main() -> Result<(), Box<dyn Error>> {
    let _telemetry = zeno_api::telemetry::init();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> Result<(), Box<dyn Error>> {
    let command = env::args().nth(1);
    if command.as_deref() == Some("seed-demo") {
        ensure_demo_seed_is_local()?;
    }
    let pool = connect_database().await?;
    match command.as_deref() {
        Some("worker") => run_worker(pool).await?,
        Some("worker-once") => run_worker_once(&pool).await?,
        Some("worker-health") => check_worker_health(&pool).await?,
        Some("migrate") => {
            sqlx::migrate!().run(&pool).await?;
            tracing::info!("database migrations applied");
        }
        Some("seed-demo") => {
            zeno_api::seed::seed_demo(&pool).await?;
            tracing::info!("local demo event seeded without replacing existing inventory");
        }
        Some("serve") | None => serve(pool).await?,
        Some(_) => {
            return Err(
                "usage: zeno-api [serve|migrate|seed-demo|worker|worker-once|worker-health]".into(),
            );
        }
    }
    Ok(())
}

async fn run_worker(pool: PgPool) -> Result<(), Box<dyn Error>> {
    let poll_interval_ms = env::var("ZENO_WORKER_POLL_INTERVAL_MS")
        .unwrap_or_else(|_| "1000".to_owned())
        .parse::<u64>()?;
    if !(100..=60_000).contains(&poll_interval_ms) {
        return Err("ZENO_WORKER_POLL_INTERVAL_MS must be between 100 and 60000".into());
    }
    let batch_size = env::var("ZENO_WORKER_BATCH_SIZE")
        .unwrap_or_else(|_| "100".to_owned())
        .parse::<u32>()?;
    if !(1..=10_000).contains(&batch_size) {
        return Err("ZENO_WORKER_BATCH_SIZE must be between 1 and 10000".into());
    }
    let cleanup_batch_size = env::var("ZENO_IDEMPOTENCY_CLEANUP_BATCH_SIZE")
        .unwrap_or_else(|_| "1000".to_owned())
        .parse::<u32>()?;
    if !(1..=10_000).contains(&cleanup_batch_size) {
        return Err("ZENO_IDEMPOTENCY_CLEANUP_BATCH_SIZE must be between 1 and 10000".into());
    }
    let cleanup_interval_ms = env::var("ZENO_IDEMPOTENCY_CLEANUP_INTERVAL_MS")
        .unwrap_or_else(|_| "3600000".to_owned())
        .parse::<u64>()?;
    if !(60_000..=86_400_000).contains(&cleanup_interval_ms) {
        return Err(
            "ZENO_IDEMPOTENCY_CLEANUP_INTERVAL_MS must be between 60000 and 86400000".into(),
        );
    }
    let instance_id = worker_instance_id();

    tracing::info!(
        poll_interval_ms,
        batch_size,
        cleanup_interval_ms,
        cleanup_batch_size,
        instance_id,
        "reservation expiry worker started"
    );
    let mut ticker = tokio::time::interval(Duration::from_millis(poll_interval_ms));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut next_cleanup_at = tokio::time::Instant::now();
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                tracing::info!("reservation expiry worker shutting down");
                return Ok(());
            }
            _ = ticker.tick() => {
                let batch_started_at = std::time::Instant::now();
                let mut succeeded = match zeno_api::worker::expire_holds_once(&pool, batch_size).await {
                    Ok(batch) => {
                        if batch.reservations_expired > 0 {
                            tracing::info!(
                                reservations_expired = batch.reservations_expired,
                                tickets_released = batch.tickets_released,
                                "expired reservation holds"
                            );
                        }
                        true
                    }
                    Err(error) => {
                        tracing::error!(
                            %error,
                            failure = zeno_api::db_error::classify(&error),
                            "reservation expiry batch failed"
                        );
                        false
                    }
                };
                if succeeded && tokio::time::Instant::now() >= next_cleanup_at {
                    match zeno_api::worker::purge_expired_idempotency_keys_once(&pool, cleanup_batch_size).await {
                        Ok(deleted) => {
                            if deleted > 0 {
                                tracing::info!(idempotency_keys_deleted = deleted, "purged expired idempotency keys");
                            }
                            next_cleanup_at = tokio::time::Instant::now() + Duration::from_millis(cleanup_interval_ms);
                        }
                        Err(error) => {
                            tracing::error!(
                                %error,
                                failure = zeno_api::db_error::classify(&error),
                                "idempotency cleanup batch failed"
                            );
                            succeeded = false;
                        }
                    }
                }
                if let Err(error) = zeno_api::worker::record_heartbeat(
                    &pool,
                    &instance_id,
                    succeeded,
                    Some(elapsed_millis(batch_started_at)),
                ).await {
                    tracing::error!(
                        %error,
                        failure = zeno_api::db_error::classify(&error),
                        "reservation expiry heartbeat update failed"
                    );
                }
            }
        }
    }
}

async fn run_worker_once(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let batch_size = env::var("ZENO_WORKER_BATCH_SIZE")
        .unwrap_or_else(|_| "100".to_owned())
        .parse::<u32>()?;
    if !(1..=10_000).contains(&batch_size) {
        return Err("ZENO_WORKER_BATCH_SIZE must be between 1 and 10000".into());
    }
    let cleanup_batch_size = env::var("ZENO_IDEMPOTENCY_CLEANUP_BATCH_SIZE")
        .unwrap_or_else(|_| "1000".to_owned())
        .parse::<u32>()?;
    if !(1..=10_000).contains(&cleanup_batch_size) {
        return Err("ZENO_IDEMPOTENCY_CLEANUP_BATCH_SIZE must be between 1 and 10000".into());
    }

    let instance_id = worker_instance_id();
    let started_at = std::time::Instant::now();
    let batch = match zeno_api::worker::expire_holds_once(pool, batch_size).await {
        Ok(batch) => batch,
        Err(error) => {
            zeno_api::worker::record_heartbeat(
                pool,
                &instance_id,
                false,
                Some(elapsed_millis(started_at)),
            )
            .await?;
            return Err(error.into());
        }
    };
    let idempotency_keys_deleted =
        match zeno_api::worker::purge_expired_idempotency_keys_once(pool, cleanup_batch_size).await
        {
            Ok(deleted) => deleted,
            Err(error) => {
                zeno_api::worker::record_heartbeat(
                    pool,
                    &instance_id,
                    false,
                    Some(elapsed_millis(started_at)),
                )
                .await?;
                return Err(error.into());
            }
        };

    zeno_api::worker::record_heartbeat(pool, &instance_id, true, Some(elapsed_millis(started_at)))
        .await?;
    tracing::info!(
        reservations_expired = batch.reservations_expired,
        tickets_released = batch.tickets_released,
        batch_size,
        idempotency_keys_deleted,
        cleanup_batch_size,
        duration_ms = started_at.elapsed().as_millis() as u64,
        instance_id,
        "reservation expiry batch completed"
    );
    Ok(())
}

async fn check_worker_health(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    let poll_interval_ms = env::var("ZENO_WORKER_POLL_INTERVAL_MS")
        .unwrap_or_else(|_| "1000".to_owned())
        .parse::<u64>()?;
    if !(100..=60_000).contains(&poll_interval_ms) {
        return Err("ZENO_WORKER_POLL_INTERVAL_MS must be between 100 and 60000".into());
    }
    let max_age_ms = poll_interval_ms.saturating_mul(3).saturating_add(5_000);
    let healthy = zeno_api::worker::is_healthy(pool, &worker_instance_id(), max_age_ms).await?;
    if !healthy {
        return Err("worker heartbeat is stale or the last expiry batch failed".into());
    }
    Ok(())
}

fn elapsed_millis(started_at: std::time::Instant) -> i64 {
    i64::try_from(started_at.elapsed().as_millis()).unwrap_or(i64::MAX)
}

fn worker_instance_id() -> String {
    env::var("ZENO_WORKER_INSTANCE_ID")
        .or_else(|_| env::var("HOSTNAME"))
        .unwrap_or_else(|_| "local-worker".to_owned())
}

fn ensure_demo_seed_is_local() -> Result<(), Box<dyn Error>> {
    let database_url = env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL must point to the local development database")?;
    let host = PgConnectOptions::from_str(&database_url)?
        .get_host()
        .to_owned();
    ensure_demo_seed_host_is_local(&host)?;
    Ok(())
}

fn ensure_demo_seed_host_is_local(host: &str) -> Result<(), Box<dyn Error>> {
    if !is_loopback_host(host) {
        return Err("refusing to seed demo data into a non-local database".into());
    }
    Ok(())
}

async fn connect_database() -> Result<PgPool, Box<dyn Error>> {
    let database_url = env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL must point to the local development database")?;
    let host = PgConnectOptions::from_str(&database_url)?
        .get_host()
        .to_owned();
    let remote_access_allowed = env::var("ZENO_ALLOW_REMOTE_DATABASE").as_deref() == Ok("true");
    if !is_loopback_host(&host) && !remote_access_allowed {
        return Err(
            "refusing a non-local DATABASE_URL; set ZENO_ALLOW_REMOTE_DATABASE=true to opt in"
                .into(),
        );
    }
    let max_connections = env::var("DATABASE_MAX_CONNECTIONS")
        .unwrap_or_else(|_| "20".to_owned())
        .parse::<u32>()?;
    if !(1..=100).contains(&max_connections) {
        return Err("DATABASE_MAX_CONNECTIONS must be between 1 and 100".into());
    }
    let acquire_timeout_seconds = database_acquire_timeout_seconds(
        &env::var("DATABASE_ACQUIRE_TIMEOUT_SECONDS").unwrap_or_else(|_| "15".to_owned()),
    )?;

    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(acquire_timeout_seconds))
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET lock_timeout = '2s'")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SET statement_timeout = '5s'")
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&database_url)
        .await?;
    Ok(pool)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn database_acquire_timeout_seconds(value: &str) -> Result<u64, std::io::Error> {
    let seconds = value.parse::<u64>().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "DATABASE_ACQUIRE_TIMEOUT_SECONDS must be a whole number between 1 and 60",
        )
    })?;
    if !(1..=60).contains(&seconds) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "DATABASE_ACQUIRE_TIMEOUT_SECONDS must be between 1 and 60",
        ));
    }
    Ok(seconds)
}

async fn serve(pool: PgPool) -> Result<(), Box<dyn Error>> {
    let bind_address = env::var("ZENO_API_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse::<SocketAddr>()?;
    let listener = TcpListener::bind(bind_address).await?;
    let auth = NeonJwtVerifier::from_env()?;
    // Absent is tolerated so the rest of the API keeps serving, but order
    // confirmation then fails closed rather than issuing tickets whose QR codes
    // could never be shown again. Logged loudly because it is a silent feature
    // outage otherwise.
    let credential_key = match zeno_api::credential::CredentialKey::from_env() {
        Ok(key) => Some(key),
        Err(_) => {
            tracing::warn!(
                "ZENO_TICKET_CREDENTIAL_KEY is not set; order confirmation and ticket issuance are disabled"
            );
            None
        }
    };
    let router = app::router_with_credentials(pool, auth, credential_key);
    tracing::info!(address = %listener.local_addr()?, "Zeno API listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() {
    let mut terminate =
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                tracing::error!(%error, "failed to listen for termination signal");
                wait_for_ctrl_c().await;
                return;
            }
        };
    tokio::select! {
        result = tokio::signal::ctrl_c() => {
            if let Err(error) = result {
                tracing::error!(%error, "failed to listen for interrupt signal");
            }
        }
        _ = terminate.recv() => tracing::info!("termination signal received"),
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    wait_for_ctrl_c().await;
}

async fn wait_for_ctrl_c() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to listen for shutdown signal");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        database_acquire_timeout_seconds, ensure_demo_seed_host_is_local, is_loopback_host,
    };

    #[test]
    fn local_database_guard_accepts_only_loopback_hosts() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("example.neon.tech"));
    }

    #[test]
    fn demo_seed_requires_loopback_even_when_remote_access_is_enabled() {
        assert!(ensure_demo_seed_host_is_local("localhost").is_ok());
        assert!(ensure_demo_seed_host_is_local("remote.example.invalid").is_err());
    }

    #[test]
    fn database_acquire_timeout_is_bounded_for_cold_start_recovery() {
        assert_eq!(database_acquire_timeout_seconds("1").unwrap(), 1);
        assert_eq!(database_acquire_timeout_seconds("15").unwrap(), 15);
        assert_eq!(database_acquire_timeout_seconds("60").unwrap(), 60);
        assert!(database_acquire_timeout_seconds("0").is_err());
        assert!(database_acquire_timeout_seconds("61").is_err());
        assert!(database_acquire_timeout_seconds("not-a-number").is_err());
    }
}

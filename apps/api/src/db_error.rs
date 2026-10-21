//! Central recording for database failures that callers collapse into an opaque
//! "service unavailable" response.
//!
//! Domain modules deliberately erase `sqlx::Error` so no SQL detail can reach a
//! client. That erasure is correct for the response but destroys the only signal
//! operators have: a pool exhaustion, a `lock_timeout` (55P03), and a
//! `statement_timeout` (57014) all surface as the same 503. Recording the failure
//! with its SQLSTATE keeps those three diagnosable from logs alone, while the
//! caller still returns a safe, stable error code.

/// Classify a SQLSTATE so saturation and lock contention can be alerted on
/// separately from genuine defects such as a missing row or a check violation.
pub fn classify(error: &sqlx::Error) -> &'static str {
    if let Some(database_error) = error.as_database_error() {
        return database_error
            .code()
            .map(|code| classify_code(&code))
            .unwrap_or("other");
    }
    // Pool and transport failures never reach PostgreSQL, so they carry no
    // SQLSTATE. They are the first thing to look for when every request is
    // timing out, so they need a name of their own rather than "unknown".
    match error {
        sqlx::Error::PoolTimedOut => "pool_timed_out",
        sqlx::Error::PoolClosed => "pool_closed",
        sqlx::Error::Io(_) => "io_failure",
        sqlx::Error::Tls(_) => "tls_failure",
        sqlx::Error::Protocol(_) => "protocol_error",
        _ => "unknown",
    }
}

fn classify_code(code: &str) -> &'static str {
    match code {
        // The per-connection lock_timeout/statement_timeout set at startup.
        "55P03" => "lock_not_available",
        "57014" => "query_canceled",
        // Pool or server connection limits, or a compute that dropped out.
        "53300" => "too_many_connections",
        "08006" | "08001" | "08003" | "08004" | "08000" => "connection_failure",
        // Serialization and deadlock retries surface here under concurrency.
        "40001" => "serialization_failure",
        "40P01" => "deadlock_detected",
        // The row the caller expected is absent; a bug if the caller assumes it.
        "P0002" => "no_data_found",
        _ => "other",
    }
}

/// Record a database failure on the current request span and return it unchanged
/// so call sites keep their existing `map_err` control flow.
pub(crate) fn record(error: sqlx::Error, operation: &'static str) -> sqlx::Error {
    let database_error = error.as_database_error();
    tracing::error!(
        operation,
        failure = classify(&error),
        constraint = database_error.and_then(|error| error.constraint()),
        table = database_error.and_then(|error| error.table()),
        "database operation failed"
    );
    error
}

/// Record the failure and return the caller-supplied safe error instead.
///
/// Domain modules must not leak `sqlx::Error` to a client, so they collapse every
/// database failure into one opaque variant. This keeps that guarantee while
/// leaving a diagnosable log line behind.
pub(crate) fn record_as<T>(error: sqlx::Error, operation: &'static str, replacement: T) -> T {
    record(error, operation);
    replacement
}

#[cfg(test)]
mod tests {
    use super::{classify, classify_code};

    #[test]
    fn saturation_and_lock_errors_are_classified_distinctly() {
        assert_eq!(classify_code("55P03"), "lock_not_available");
        assert_eq!(classify_code("57014"), "query_canceled");
        assert_eq!(classify_code("53300"), "too_many_connections");
        assert_eq!(classify_code("08006"), "connection_failure");
        assert_eq!(classify_code("40P01"), "deadlock_detected");
        assert_eq!(classify_code("P0002"), "no_data_found");
    }

    #[test]
    fn unrecognised_codes_fall_back_without_hiding_the_failure() {
        assert_eq!(classify_code("23505"), "other");
        assert_eq!(classify_code("not-a-code"), "other");
    }

    #[test]
    fn non_database_errors_are_classified_as_unknown() {
        assert_eq!(classify(&sqlx::Error::RowNotFound), "unknown");
        assert_eq!(classify(&sqlx::Error::Configuration("x".into())), "unknown");
    }

    #[test]
    fn pool_saturation_is_named_rather_than_reported_as_unknown() {
        // A cold-start or exhausted pool is the first thing to check when every
        // request times out, so it must not collapse into "unknown".
        assert_eq!(classify(&sqlx::Error::PoolTimedOut), "pool_timed_out");
        assert_eq!(classify(&sqlx::Error::PoolClosed), "pool_closed");
        assert_eq!(
            classify(&sqlx::Error::Io(std::io::Error::other("boom"))),
            "io_failure"
        );
    }
}

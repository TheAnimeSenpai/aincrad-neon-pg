//! Neon-compatible Postgres connection pool builder.
//!
//! Key behaviour: `statement_cache_capacity(0)` is set on every connection so
//! the pool works correctly with Neon's PgBouncer endpoint in transaction mode.
//! Without this, prepared-statement IDs mismatch across pool connections.

use std::str::FromStr;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// Pool configuration values.
pub struct NeonPgConfig {
    /// Full Postgres connection string.
    pub database_url: String,
    /// Maximum number of connections to maintain in the pool.
    pub max_connections: u32,
    /// Seconds a connection can sit idle before being closed.
    pub idle_timeout_secs: u64,
    /// Seconds to wait when acquiring a connection before giving up.
    pub connect_timeout_secs: u64,
    /// Statement timeout injected on every new connection, e.g. `"30s"`.
    pub statement_timeout: String,
}

/// Build a lazy Neon-compatible `PgPool`.
///
/// The pool is created with prepared-statement caching disabled (required for
/// Neon PgBouncer transaction mode) and a `SET statement_timeout` hook applied
/// on each new connection.
pub fn create_pool(cfg: NeonPgConfig) -> Result<PgPool, sqlx::Error> {
    let statement_timeout = cfg.statement_timeout.clone();
    let connect_options =
        PgConnectOptions::from_str(&cfg.database_url)?.statement_cache_capacity(0);
    Ok(PgPoolOptions::new()
        .max_connections(cfg.max_connections)
        .idle_timeout(Some(Duration::from_secs(cfg.idle_timeout_secs)))
        .acquire_timeout(Duration::from_secs(cfg.connect_timeout_secs))
        .after_connect(move |conn, _| {
            let timeout = statement_timeout.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET statement_timeout = '{timeout}'"))
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect_lazy_with(connect_options))
}

/// Cheap liveness check — `SELECT 1`.
pub async fn healthcheck(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(pool)
        .await
        .map(|_| ())
}

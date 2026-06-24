//! Postgres RLS transaction helpers for multi-tenant apps.
//!
//! Sets `app.current_tenant_id` or `app.bypass_rls` as a session-local
//! variable on every transaction so Postgres row-level security policies
//! can enforce tenant isolation transparently.

use std::{future::Future, pin::Pin};

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Pinned, boxed async closure return type for scope helpers.
///
/// `E` is the application error type; it must implement `From<sqlx::Error>`
/// so transaction setup failures propagate cleanly.
pub type ScopeFuture<'a, T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// Run `f` inside a transaction with the tenant id bound as a transaction-local
/// setting (`app.current_tenant_id`).
///
/// Activates row-level security policies that filter by tenant. The
/// transaction is committed on success and rolled back on error.
///
/// Uses `set_config(name, value, is_local => true)` rather than
/// `SET LOCAL app.current_tenant_id = $1`: the `SET` command is parsed before
/// parameter binding, so a bind placeholder (`$1`) is a syntax error. The
/// `set_config(..., true)` form is the parameterizable equivalent of
/// `SET LOCAL` and is read back identically via `current_setting(...)`.
pub async fn with_tenant_scope<T, E, F>(pool: &PgPool, tenant_id: &Uuid, f: F) -> Result<T, E>
where
    E: From<sqlx::Error>,
    F: for<'a> FnOnce(&'a mut Transaction<'_, Postgres>) -> ScopeFuture<'a, T, E>,
{
    let mut tx = pool.begin().await.map_err(E::from)?;

    sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(E::from)?;

    let result = f(&mut tx).await;

    if result.is_ok() {
        tx.commit().await.map_err(E::from)?;
    }

    result
}

/// Run `f` inside a transaction with `SET LOCAL app.bypass_rls = 'on'`.
///
/// Bypasses all RLS tenant-isolation policies. The bypass is scoped to this
/// transaction and cannot leak to other connections.
///
/// # Safety
///
/// Must only be called from handlers gated by platform-level RBAC (e.g.
/// `super_admin`). Calling from a tenant-scoped handler defeats RLS isolation.
pub async fn with_cross_tenant_scope<T, E, F>(pool: &PgPool, f: F) -> Result<T, E>
where
    E: From<sqlx::Error>,
    F: for<'a> FnOnce(&'a mut Transaction<'_, Postgres>) -> ScopeFuture<'a, T, E>,
{
    let mut tx = pool.begin().await.map_err(E::from)?;

    sqlx::query("SET LOCAL app.bypass_rls = 'on'")
        .execute(&mut *tx)
        .await
        .map_err(E::from)?;

    let result = f(&mut tx).await;

    if result.is_ok() {
        tx.commit().await.map_err(E::from)?;
    }

    result
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use sqlx::Row;
    use sqlx::postgres::PgPoolOptions;

    // Regression guard for the `SET LOCAL ... = $1` bug: a bind placeholder is a
    // syntax error in a `SET` statement (parsed before parameter binding), so
    // every tenant-scoped transaction failed at runtime. `set_config(..., true)`
    // is the parameterizable equivalent. `#[ignore]` — needs a reachable PG:
    //
    //   DATABASE_URL=postgres://gakuin:gakuin@localhost:5433/gakuin_dev \
    //     cargo test -p aincrad-neon-pg -- --ignored
    #[tokio::test]
    #[ignore = "requires a reachable Postgres"]
    async fn with_tenant_scope_binds_current_tenant_id() {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://gakuin:gakuin@localhost:5433/gakuin_dev".to_string());
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("Postgres should be reachable");

        let tenant = Uuid::new_v4();
        let read_back: String = with_tenant_scope(&pool, &tenant, |tx| {
            Box::pin(async move {
                let row = sqlx::query("SELECT current_setting('app.current_tenant_id') AS v")
                    .fetch_one(&mut **tx)
                    .await?;
                Ok::<String, sqlx::Error>(row.get("v"))
            })
        })
        .await
        .expect("tenant scope should set app.current_tenant_id without a syntax error");

        assert_eq!(read_back, tenant.to_string());
    }
}

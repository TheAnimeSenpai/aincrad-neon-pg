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

/// Run `f` inside a transaction with `SET LOCAL app.current_tenant_id = $1`.
///
/// Activates row-level security policies that filter by tenant. The
/// transaction is committed on success and rolled back on error.
pub async fn with_tenant_scope<T, E, F>(pool: &PgPool, tenant_id: &Uuid, f: F) -> Result<T, E>
where
    E: From<sqlx::Error>,
    F: for<'a> FnOnce(&'a mut Transaction<'_, Postgres>) -> ScopeFuture<'a, T, E>,
{
    let mut tx = pool.begin().await.map_err(E::from)?;

    sqlx::query("SET LOCAL app.current_tenant_id = $1")
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

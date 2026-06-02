# aincrad-neon-pg

Neon-compatible Postgres pool builder and RLS transaction helpers.

## Usage

```toml
[dependencies]
aincrad-neon-pg = { git = "https://github.com/TheAnimeSenpai/aincrad-neon-pg" }
```

### Pool

```rust
use aincrad_neon_pg::{NeonPgConfig, create_pool, healthcheck};

let pool = create_pool(NeonPgConfig {
    database_url: env::var("DATABASE_URL")?,
    max_connections: 10,
    idle_timeout_secs: 300,
    connect_timeout_secs: 5,
    statement_timeout: "30s".into(),
})?;

healthcheck(&pool).await?;
```

`statement_cache_capacity(0)` is set on every connection — required for Neon's PgBouncer endpoint in transaction mode. Without it, prepared-statement IDs mismatch across pool connections.

### RLS tenant scope

```rust
use aincrad_neon_pg::{with_tenant_scope, with_cross_tenant_scope};

// Sets SET LOCAL app.current_tenant_id = $tenant_id for the transaction.
let result = with_tenant_scope(&pool, &tenant_id, |tx| Box::pin(async move {
    sqlx::query_as!(Row, "SELECT * FROM items").fetch_all(&mut **tx).await
})).await?;

// Sets SET LOCAL app.bypass_rls = 'on' — platform/admin only.
let result = with_cross_tenant_scope(&pool, |tx| Box::pin(async move {
    sqlx::query_as!(Row, "SELECT * FROM items").fetch_all(&mut **tx).await
})).await?;
```

Transactions are committed on success and rolled back on error. The RLS variable is scoped to the transaction and cannot leak.

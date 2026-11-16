//! The schema versions and their migrations (design 3.5, 3.6).

/// The newest schema version this crate reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// One migration: the SQL that takes the schema to version `to`.
pub struct Migration {
    /// The version after this migration.
    pub to: u32,
    /// The migration's SQL.
    pub sql: &'static str,
}

/// Every migration, in order.
pub const MIGRATIONS: &[Migration] = &[Migration {
    to: 1,
    sql: include_str!("../migrations/0001_initial.sql"),
}];

/// Checks that a migration list goes 1, 2, 3, ... with no gap.
pub(crate) fn check_list(migrations: &[Migration]) -> bool {
    !migrations.is_empty()
        && migrations
            .iter()
            .zip(1_u32..)
            .all(|(migration, to)| migration.to == to)
}

/// Runs one migration in one transaction: its SQL, `user_version`, `meta.schema_version` and one
/// `schema.migrated` ledger row dated `at_ms`, which the caller has floored at the last commit
/// (3.6). The first migration of a new store also writes `StoreCreated`.
pub(crate) fn run(
    connection: &mut rusqlite::Connection,
    migration: &Migration,
    from: u32,
    at_ms: i64,
    creation: Option<(String, crate::key::KeyKind)>,
) -> Result<(), crate::tx::StoreFault> {
    use crate::mutation::Mutation;
    use crate::tx::{Actor, OpMeta, set_meta, sql_fault};

    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| sql_fault("begin migration", &e))?;
    tx.execute_batch(migration.sql)
        .map_err(|e| sql_fault("migrate", &e))?;
    tx.execute_batch(&format!("PRAGMA user_version = {}", migration.to))
        .map_err(|e| sql_fault("migrate", &e))?;
    set_meta(&tx, "schema_version", &migration.to.to_string())?;
    let mut mutations = vec![Mutation::SchemaMigrated {
        from,
        to: migration.to,
    }];
    if let Some((store_id, key_kind)) = creation {
        mutations.push(Mutation::StoreCreated {
            store_id,
            key_kind,
            created_at_ms: at_ms,
        });
    }
    for mutation in &mutations {
        crate::apply::apply(&tx, mutation)?;
    }
    let meta = OpMeta {
        operation_type: "schema.migrated",
        actor: Actor::broker(),
        idempotency: None,
        causation_id: None,
        correlation_id: None,
        now_ms: at_ms,
    };
    crate::ledger::append(&tx, &meta, &mutations, None)?;
    tx.commit().map_err(|e| sql_fault("commit migration", &e))
}

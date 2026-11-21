//! SQL for capacity rows and reservations.

use sqlx::{PgConnection, PgPool};

use super::NodeMetrics;

pub async fn store_node(db: &PgPool, node: &str, m: &NodeMetrics) -> Result<(), sqlx::Error> {
    let i = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
    sqlx::query!(
        "insert into garage_capacity (node, data_avail, data_total, meta_avail, meta_total, replication, checked_at)
         values ($1, $2, $3, $4, $5, $6, now())
         on conflict (node) do update set data_avail = excluded.data_avail, data_total = excluded.data_total,
           meta_avail = excluded.meta_avail, meta_total = excluded.meta_total,
           replication = excluded.replication, checked_at = now()",
        node,
        i(m.data_avail),
        i(m.data_total),
        m.meta_avail.map(i),
        m.meta_total.map(i),
        i16::try_from(m.replication).unwrap_or(2),
    )
    .execute(db)
    .await?;
    Ok(())
}

pub struct NodeRow {
    pub node: String,
    pub data_avail: i64,
    pub data_total: i64,
    pub meta_avail: Option<i64>,
    pub meta_total: Option<i64>,
    pub replication: i16,
    pub fresh: bool,
}

pub async fn nodes(db: &mut PgConnection, fresh_secs: f64) -> Result<Vec<NodeRow>, sqlx::Error> {
    sqlx::query_as!(
        NodeRow,
        r#"select node, data_avail, data_total, meta_avail, meta_total, replication,
                  checked_at > now() - make_interval(secs => $1) as "fresh!"
           from garage_capacity order by node"#,
        fresh_secs
    )
    .fetch_all(db)
    .await
}

/// Bytes reserved by uploads whose transcode has not ended.
pub async fn reserved(db: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"select coalesce(sum(reserved_bytes), 0)::bigint as "n!" from uploads where reserved_bytes > 0"#
    )
    .fetch_one(db)
    .await
}

/// Serialises admission across replicas for the rest of the transaction.
pub async fn lock_admission(db: &mut PgConnection) -> Result<(), sqlx::Error> {
    // Any fixed key unique to this purpose. 0x5216_A000 reads as "r16a admission".
    sqlx::query!("select pg_advisory_xact_lock(1377214464)")
        .execute(db)
        .await?;
    Ok(())
}

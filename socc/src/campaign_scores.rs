use std::time::Duration;

use deadpool_redis::{redis::AsyncCommands, Connection};
use records_lib::{redis_key::mappacks_key, update_mappacks::update_mappack};
use sqlx::{pool::PoolConnection, MySql};

const PROCESS_DURATION_SECS: u64 = 3600 * 24; // Every day
pub const PROCESS_DURATION: Duration = Duration::from_secs(PROCESS_DURATION_SECS);

pub async fn update(
    mut mysql_conn: PoolConnection<MySql>,
    mut redis_conn: Connection,
) -> anyhow::Result<()> {
    let mappacks: Vec<String> = redis_conn.smembers(mappacks_key()).await?;

    if mappacks.is_empty() {
        tracing::warn!("No mappacks to update");
        return Ok(());
    }

    tracing::info!(
        "Updating mappacks: {}",
        mappacks
            .iter()
            .map(|s| format!("`{s}`"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    for mappack_id in &mappacks {
        update_mappack(mappack_id, &mut mysql_conn, &mut redis_conn).await?;
    }

    Ok(())
}

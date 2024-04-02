use std::time::Duration;

use deadpool_redis::{redis::AsyncCommands, Connection};
use futures::{future, stream, StreamExt as _, TryStreamExt};
use records_lib::{
    event,
    redis_key::{mappack_key, mappacks_key},
    update_mappacks::update_mappack,
    MySqlPool,
};
use sqlx::{pool::PoolConnection, MySql};

const PROCESS_DURATION_SECS: u64 = 3600 * 24; // Every day
pub const PROCESS_DURATION: Duration = Duration::from_secs(PROCESS_DURATION_SECS);

async fn event_mappacks(
    mysql_pool: &MySqlPool,
    redis_conn: &mut Connection,
) -> anyhow::Result<Vec<String>> {
    let events = event::event_list(mysql_pool).await?;
    let events_n = events.len();
    let events_maps = stream::iter(events)
        .map(|event| async move {
            let editions = event::event_editions_list(mysql_pool, &event.handle).await?;
            let editions_n = editions.len();
            let editions = stream::iter(editions)
                // Event editions bound with an MX ID will update their scores with
                // the regular process
                .filter(|edition| future::ready(edition.mx_id.is_none()))
                .map(|edition| async move {
                    event::event_edition_maps(mysql_pool, edition.event_id, edition.id)
                        .await
                        .map(|maps| {
                            (
                                format!("__{}__{}__", edition.event_id, edition.id),
                                maps.into_iter().collect::<Vec<_>>(),
                            )
                        })
                })
                .buffer_unordered(editions_n)
                .try_collect::<Vec<_>>()
                .await?;
            anyhow::Ok(editions.into_iter().collect::<Vec<_>>())
        })
        .buffer_unordered(events_n)
        .try_collect::<Vec<_>>()
        .await?;

    let mut out = Vec::new();

    for (mappack_id, maps) in events_maps.into_iter().flatten() {
        for map in maps {
            redis_conn
                .sadd(mappack_key(&mappack_id), map.game_id)
                .await?;
        }
        redis_conn.sadd(mappacks_key(), &mappack_id).await?;
        out.push(mappack_id);
    }

    Ok(out)
}

pub async fn update(
    mysql_pool: MySqlPool,
    mut mysql_conn: PoolConnection<MySql>,
    mut redis_conn: Connection,
) -> anyhow::Result<()> {
    let mappacks: Vec<String> = redis_conn.smembers(mappacks_key()).await?;
    let mappacks = mappacks
        .into_iter()
        .chain(event_mappacks(&mysql_pool, &mut redis_conn).await?);

    for mappack_id in mappacks {
        update_mappack(&mappack_id, &mut mysql_conn, &mut redis_conn).await?;
    }

    Ok(())
}

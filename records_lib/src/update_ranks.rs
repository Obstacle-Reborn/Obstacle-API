use deadpool_redis::redis::AsyncCommands;
use sqlx::{Executor, MySql, MySqlConnection};

use crate::{
    error::RecordsResult,
    models,
    redis_key::{map_key, MapKey},
    GetSqlFragments, RedisConnection,
};

async fn count_records_map<'c, E: Executor<'c, Database = MySql>>(
    db: E,
    map_id: u32,
) -> RecordsResult<i64> {
    sqlx::query_scalar(
        "SELECT COUNT(*)
        FROM (SELECT * FROM records
        WHERE map_id = ?
        GROUP BY record_player_id) r",
    )
    .bind(map_id)
    .fetch_one(db)
    .await
    .map_err(Into::into)
}

/// Checks if the Redis leaderboard for the map with the `key` has a different count
/// that in the database, and reupdates the Redis leaderboard completly if so.
///
/// This is a check to avoid records duplicates, that may happen sometimes.
///
/// It returns the number of records in the map.
pub async fn update_leaderboard(
    (db, redis_conn): (&mut MySqlConnection, &mut RedisConnection),
    models::Map {
        id: map_id,
        reversed,
        ..
    }: &models::Map,
    event: Option<&(models::Event, models::EventEdition)>,
) -> RecordsResult<i64> {
    let reversed_lb = reversed.unwrap_or(false);
    let key = map_key(*map_id, event);
    let redis_count: i64 = redis_conn.zcount(&key, "-inf", "+inf").await?;
    let mysql_count: i64 = count_records_map(&mut *db, *map_id).await?;

    let (join_event, and_event) = event.get_sql_fragments();

    if redis_count != mysql_count {
        let query = format!(
            "SELECT record_player_id, {}(time) AS time
            FROM records r
            {join_event}
                WHERE map_id = ?
                    {and_event}
                GROUP BY record_player_id
                ORDER BY time {}, record_date ASC",
            if reversed_lb { "MAX" } else { "MIN" },
            if reversed_lb { "DESC" } else { "ASC" },
        );

        let mut query = sqlx::query_as(&query).bind(map_id);

        if let Some((event, edition)) = event {
            query = query.bind(event.id).bind(edition.id);
        }

        let all_map_records: Vec<(u32, i32)> = query.fetch_all(db).await?;

        let _removed_count: i64 = redis_conn.del(&key).await?;

        for record in all_map_records {
            let _: i64 = redis_conn.zadd(&key, record.0, record.1).await?;
        }
    }

    Ok(mysql_count)
}

/// Get the rank of a time in a map, or fully updates its leaderboard if not found.
///
/// The full update means a delete of the Redis key then a reinsertion of all the records.
/// This may be called when the SQL and Redis databases had the same amount of records on a map,
/// but the times were not corresponding. It generally happens after a database migration.
pub async fn get_rank_or_full_update(
    (db, redis_conn): (&mut MySqlConnection, &mut RedisConnection),
    map @ models::Map {
        id: map_id,
        reversed,
        ..
    }: &models::Map,
    time: i32,
    event: Option<&(models::Event, models::EventEdition)>,
) -> RecordsResult<i32> {
    async fn get_rank(
        redis_conn: &mut RedisConnection,
        key: &MapKey<'_>,
        time: i32,
        reversed: bool,
    ) -> RecordsResult<Option<i32>> {
        let player_id: Vec<u32> = if reversed {
            redis_conn.zrevrangebyscore_limit(key, time, time, 0, 1)
        } else {
            redis_conn.zrangebyscore_limit(key, time, time, 0, 1)
        }
        .await?;

        match player_id.first() {
            Some(id) => {
                let rank: i32 = if reversed {
                    redis_conn.zrevrank(key, id)
                } else {
                    redis_conn.zrank(key, id)
                }
                .await?;
                Ok(Some(rank + 1))
            }
            None => Ok(None),
        }
    }

    let reversed = reversed.unwrap_or(false);
    let key = &map_key(*map_id, event);

    match get_rank(redis_conn, key, time, reversed).await? {
        Some(rank) => Ok(rank),
        None => {
            redis_conn.del(key).await?;
            update_leaderboard((db, redis_conn), map, event).await?;
            let rank = get_rank(redis_conn, key, time, reversed)
                .await?
                .unwrap_or_else(|| {
                    // TODO: make a more clear message showing diff
                    panic!(
                        "redis leaderboard for (`{key}`) should be updated \
                        at this point"
                    )
                });
            Ok(rank)
        }
    }
}

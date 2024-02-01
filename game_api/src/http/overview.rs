use actix_web::{
    web::{Data, Query},
    Responder,
};
use deadpool_redis::{redis::AsyncCommands, Connection as RedisConnection};
use futures::StreamExt;
use records_lib::{
    models::{self, Map},
    redis_key::map_key,
    update_ranks::{get_rank_or_full_update, update_leaderboard},
    Database, GetSqlFragments,
};
use serde::{Deserialize, Serialize};
use sqlx::MySqlConnection;
use tracing_actix_web::RequestId;

use crate::{utils::json, FitRequestId, RecordsResponse, RecordsResult, RecordsResultExt};

#[derive(Deserialize)]
pub struct OverviewQuery {
    #[serde(alias = "playerId")]
    login: String,
    #[serde(alias = "mapId")]
    map_uid: String,
}

pub type OverviewReq = Query<OverviewQuery>;

#[derive(sqlx::FromRow, Debug, Clone)]
pub struct RecordQueryRow {
    pub login: String,
    pub nickname: String,
    pub time: i32,
    #[sqlx(flatten)]
    pub map: Map,
}

#[derive(Debug, Clone, Deserialize, Serialize, sqlx::FromRow)]
#[serde(rename = "records")]
pub struct RankedRecord {
    pub rank: u32,
    pub login: String,
    pub nickname: String,
    pub time: i32,
}

async fn get_range(
    db: &Database,
    (mysql_conn, redis_conn): (&mut MySqlConnection, &mut RedisConnection),
    Map {
        id: map_id,
        reversed,
        ..
    }: &Map,
    (start, end): (u32, u32),
    event: Option<&(models::Event, models::EventEdition)>,
) -> RecordsResult<Vec<RankedRecord>> {
    let reversed = reversed.unwrap_or(false);
    let key = map_key(*map_id, event);

    let (join_event, and_event) = event.get_sql_fragments();

    // transforms exclusive to inclusive range
    let end = end - 1;
    let ids: Vec<i32> = if reversed {
        redis_conn.zrevrange(key, start as isize, end as isize)
    } else {
        redis_conn.zrange(key, start as isize, end as isize)
    }
    .await
    .with_api_err()?;

    if ids.is_empty() {
        // Avoids the query building to have a `AND record_player_id IN ()` fragment
        return Ok(Vec::new());
    }

    let params = ids
        .iter()
        .map(|_| "?".to_string())
        .collect::<Vec<String>>()
        .join(",");

    let query = format!(
        "SELECT CAST(0 AS UNSIGNED) AS rank,
            p.login AS login,
            p.name AS nickname,
            {func}(time) as time,
            m.*
        FROM records r
        {join_event}
        INNER JOIN players p ON r.record_player_id = p.id
        INNER JOIN maps m ON m.id = r.map_id
        WHERE map_id = ? AND record_player_id IN ({params})
            {and_event}
        GROUP BY record_player_id
        ORDER BY time {order}, record_date ASC",
        params = params,
        func = if reversed { "MAX" } else { "MIN" },
        order = if reversed { "DESC" } else { "ASC" },
        join_event = join_event,
        and_event = and_event,
    );

    let mut query = sqlx::query_as(&query).bind(map_id);
    for id in ids {
        query = query.bind(id);
    }

    if let Some((event, edition)) = event {
        query = query.bind(event.id).bind(edition.id);
    }

    let mut records = query.fetch(&mut *mysql_conn);
    let mut out = Vec::with_capacity(records.size_hint().0);

    let mysql_conn = &mut db.mysql_pool.acquire().await.with_api_err()?;

    while let Some(record) = records.next().await {
        let RecordQueryRow {
            login,
            nickname,
            time,
            map,
        } = record.with_api_err()?;

        out.push(RankedRecord {
            rank: get_rank_or_full_update((mysql_conn, redis_conn), &map, time, event).await? as _,
            login,
            nickname,
            time,
        });
    }

    Ok(out)
}

pub async fn overview(
    req_id: RequestId,
    db: Data<Database>,
    Query(body): Query<OverviewQuery>,
    event: Option<(String, u32)>,
) -> RecordsResponse<impl Responder> {
    let mysql_conn = &mut db.mysql_pool.acquire().await.with_api_err().fit(req_id)?;
    let redis_conn = &mut db.redis_pool.get().await.fit(req_id)?;

    let ref map @ Map {
        id,
        linked_map,
        reversed,
        ..
    } = records_lib::must::have_map(&mut **mysql_conn, &body.map_uid)
        .await
        .fit(req_id)?;
    let player_id = records_lib::must::have_player(&mut **mysql_conn, &body.login)
        .await
        .fit(req_id)?
        .id;
    let map_id = linked_map.unwrap_or(id);
    let reversed = reversed.unwrap_or(false);

    let event = match event {
        Some((event_handle, edition_id)) => Some(
            records_lib::must::have_event_edition_with_map(
                &db,
                &body.map_uid,
                event_handle,
                edition_id,
            )
            .await
            .fit(req_id)?,
        ),
        None => None,
    };

    // Update redis if needed
    let key = map_key(map_id, event.as_ref());
    let count = update_leaderboard((mysql_conn, redis_conn), map, event.as_ref())
        .await
        .fit(req_id)? as u32;

    let mut ranked_records: Vec<RankedRecord> = vec![];

    // -- Compute display ranges
    const TOTAL_ROWS: u32 = 15;
    const NO_RECORD_ROWS: u32 = TOTAL_ROWS - 1;

    let player_rank: Option<i64> = if reversed {
        redis_conn.zrevrank(&key, player_id)
    } else {
        redis_conn.zrank(&key, player_id)
    }
    .await
    .with_api_err()
    .fit(req_id)?;
    let player_rank = player_rank.map(|r| r as u64 as u32);

    if let Some(player_rank) = player_rank {
        // The player has a record and is in top ROWS, display ROWS records
        if player_rank < TOTAL_ROWS {
            let range = get_range(
                &db,
                (mysql_conn, redis_conn),
                map,
                (0, TOTAL_ROWS),
                event.as_ref(),
            )
            .await
            .fit(req_id)?;
            ranked_records.extend(range);
        }
        // The player is not in the top ROWS records, display top3 and then center around the player rank
        else {
            // push top3
            let range = get_range(&db, (mysql_conn, redis_conn), map, (0, 3), event.as_ref())
                .await
                .fit(req_id)?;
            ranked_records.extend(range);

            // the rest is centered around the player
            let row_minus_top3 = TOTAL_ROWS - 3;
            let range = {
                let start = player_rank - row_minus_top3 / 2;
                let end = player_rank + row_minus_top3 / 2;
                if end >= count {
                    (start - (end - count), count)
                } else {
                    (start, end)
                }
            };

            let range = get_range(&db, (mysql_conn, redis_conn), map, range, event.as_ref())
                .await
                .fit(req_id)?;
            ranked_records.extend(range);
        }
    }
    // The player has no record, so ROWS = ROWS - 1 to keep one last line for the player
    else {
        // There is more than ROWS record + top3,
        // So display all top ROWS records and then the last 3
        if count > NO_RECORD_ROWS {
            // top (ROWS - 1 - 3)
            let range = get_range(
                &db,
                (mysql_conn, redis_conn),
                map,
                (0, NO_RECORD_ROWS - 3),
                event.as_ref(),
            )
            .await
            .fit(req_id)?;
            ranked_records.extend(range);

            // last 3
            let range = get_range(
                &db,
                (mysql_conn, redis_conn),
                map,
                (count - 3, count),
                event.as_ref(),
            )
            .await
            .fit(req_id)?;
            ranked_records.extend(range);
        }
        // There is enough records to display them all
        else {
            let range = get_range(
                &db,
                (mysql_conn, redis_conn),
                map,
                (0, NO_RECORD_ROWS),
                event.as_ref(),
            )
            .await
            .fit(req_id)?;
            ranked_records.extend(range);
        }
    }

    #[derive(Serialize)]
    struct Response {
        response: Vec<RankedRecord>,
    }

    let response = ranked_records;
    json(Response { response })
}

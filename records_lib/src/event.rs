use sqlx::MySqlConnection;

use crate::{error::RecordsResult, models};

#[derive(serde::Serialize)]
pub struct EventListItem {
    pub handle: String,
    pub last_edition_id: i64,
}

#[derive(sqlx::FromRow)]
struct RawSqlEventListItem {
    handle: String,
    last_edition_id: u32,
}

impl From<RawSqlEventListItem> for EventListItem {
    fn from(value: RawSqlEventListItem) -> Self {
        Self {
            handle: value.handle,
            last_edition_id: value.last_edition_id as _,
        }
    }
}

pub async fn event_list(db: &mut MySqlConnection) -> RecordsResult<Vec<EventListItem>> {
    sqlx::query_as::<_, RawSqlEventListItem>(
        "select ev.handle as handle, max(ee.id) as last_edition_id from event ev
        inner join event_edition ee on ev.id = ee.event_id
        inner join event_edition_maps eem on ee.id = eem.edition_id and ee.event_id = eem.event_id
        group by ev.id, ev.handle
        order by ev.id",
    )
    .fetch_all(db)
    .await
    .map_err(From::from)
    .map(|e| e.into_iter().map(From::from).collect())
}

pub async fn event_editions_list(
    db: &mut MySqlConnection,
    event_handle: &str,
) -> RecordsResult<Vec<models::EventEdition>> {
    let res = sqlx::query_as(
        "select ee.* from event_edition ee
        inner join event e on ee.event_id = e.id
        where e.handle = ?",
    )
    .bind(event_handle)
    .fetch_all(db)
    .await?;
    Ok(res)
}

pub async fn event_edition_maps(
    db: &mut MySqlConnection,
    event_id: u32,
    edition_id: u32,
) -> RecordsResult<Vec<models::Map>> {
    sqlx::query_as(
        "select m.* from maps m
        inner join event_edition_maps eem on m.id = eem.map_id
        where eem.event_id = ? and eem.edition_id = ?",
    )
    .bind(event_id)
    .bind(edition_id)
    .fetch_all(db)
    .await
    .map_err(From::from)
}

#[inline(always)]
pub fn event_edition_key(event_id: u32, edition_id: u32) -> String {
    format!("__{event_id}__{edition_id}__")
}

pub async fn get_event_by_handle(
    db: &mut MySqlConnection,
    handle: &str,
) -> RecordsResult<Option<models::Event>> {
    let r = sqlx::query_as("SELECT * FROM event WHERE handle = ?")
        .bind(handle)
        .fetch_optional(db)
        .await?;
    Ok(r)
}

pub async fn get_edition_by_id(
    db: &mut MySqlConnection,
    event_id: u32,
    edition_id: u32,
) -> RecordsResult<Option<models::EventEdition>> {
    let r = sqlx::query_as("SELECT * FROM event_edition WHERE event_id = ? AND id = ?")
        .bind(event_id)
        .bind(edition_id)
        .fetch_optional(db)
        .await?;
    Ok(r)
}

pub async fn get_categories_by_edition_id(
    db: &mut MySqlConnection,
    event_id: u32,
    edition_id: u32,
) -> RecordsResult<Vec<models::EventCategory>> {
    let r = sqlx::query_as(
        "select ec.* from event_category ec
        inner join event_edition_categories eec on ec.id = eec.category_id
        where eec.event_id = ? and eec.edition_id = ?
        order by ec.id asc",
    )
    .bind(event_id)
    .bind(edition_id)
    .fetch_all(db)
    .await?;

    Ok(r)
}

#[derive(async_graphql::SimpleObject)]
pub struct MedalTimes {
    pub bronze_time: i32,
    pub silver_time: i32,
    pub gold_time: i32,
    pub champion_time: i32,
}

pub async fn get_medal_times_of<E: for<'c> sqlx::Executor<'c, Database = sqlx::MySql>>(
    db: E,
    event_id: u32,
    edition_id: u32,
    map_id: u32,
) -> RecordsResult<MedalTimes> {
    let (bronze_time, silver_time, gold_time, champion_time) = sqlx::query_as("
    select bronze.time, silver.time, gold.time, champion.time
    from event_edition_maps_medals bronze, event_edition_maps_medals silver, event_edition_maps_medals gold, event_edition_maps_medals champion
    where bronze.event_id = silver.event_id and silver.event_id = gold.event_id and gold.event_id = champion.event_id
        and bronze.edition_id = silver.edition_id and silver.edition_id = gold.edition_id and gold.edition_id = champion.edition_id
        and bronze.map_id = silver.map_id and silver.map_id = gold.map_id and gold.map_id = champion.map_id
        and bronze.medal_id = 1 and silver.medal_id = 2 and gold.medal_id = 3 and champion.medal_id = 4
        and bronze.map_id = ? and bronze.event_id = ? and bronze.edition_id = ?")
    .bind(map_id).bind(event_id).bind(edition_id).fetch_optional(db).await?.unwrap_or((-1, -1, -1, -1));

    Ok(MedalTimes {
        bronze_time,
        silver_time,
        gold_time,
        champion_time,
    })
}

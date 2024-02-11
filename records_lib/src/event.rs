use sqlx::MySqlConnection;

use crate::{error::RecordsResult, models};

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

pub async fn get_categories_by_edition_id<E: for<'c> sqlx::Executor<'c, Database = sqlx::MySql>>(
    db: E,
    event_id: u32,
    edition_id: u32,
) -> RecordsResult<Vec<models::EventCategory>> {
    let r = sqlx::query_as(
        "SELECT DISTINCT ec.* FROM event_edition ee
        LEFT JOIN event_edition_categories eec ON eec.edition_id = ee.id
        LEFT JOIN event_categories ecs ON ecs.event_id = ee.event_id
        INNER JOIN event_category ec ON ec.id IN (eec.category_id, ecs.category_id)
        WHERE ee.event_id = ? AND ee.id = ?
        ORDER BY ecs.category_id DESC",
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

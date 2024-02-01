use crate::{error::RecordsResult, models, Database};

pub async fn get_event_by_handle(
    db: &Database,
    handle: &str,
) -> RecordsResult<Option<models::Event>> {
    let r = sqlx::query_as("SELECT * FROM event WHERE handle = ?")
        .bind(handle)
        .fetch_optional(&db.mysql_pool)
        .await?;
    Ok(r)
}

pub async fn get_edition_by_id(
    db: &Database,
    event_id: u32,
    edition_id: u32,
) -> RecordsResult<Option<models::EventEdition>> {
    let r = sqlx::query_as("SELECT * FROM event_edition WHERE event_id = ? AND id = ?")
        .bind(event_id)
        .bind(edition_id)
        .fetch_optional(&db.mysql_pool)
        .await?;
    Ok(r)
}

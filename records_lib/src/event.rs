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

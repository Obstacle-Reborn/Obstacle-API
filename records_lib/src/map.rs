use sqlx::{Executor, MySql};

use crate::{error::RecordsResult, models::Map};

pub async fn get_map_from_game_id<'c, E: Executor<'c, Database = MySql>>(
    db: E,
    map_game_id: &str,
) -> RecordsResult<Option<Map>> {
    let r = sqlx::query_as("SELECT * FROM maps WHERE game_id = ?")
        .bind(map_game_id)
        .fetch_optional(db)
        .await?;
    Ok(r)
}

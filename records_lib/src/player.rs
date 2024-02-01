use crate::{error::RecordsResult, models::Player};

pub async fn get_player_from_login<'c, E: sqlx::Executor<'c, Database = sqlx::MySql>>(
    db: E,
    player_login: &str,
) -> RecordsResult<Option<Player>> {
    let r = sqlx::query_as("SELECT * FROM players WHERE login = ?")
        .bind(player_login)
        .fetch_optional(db)
        .await?;
    Ok(r)
}

pub async fn get_player_from_id<'c, E: sqlx::Executor<'c, Database = sqlx::MySql>>(
    db: E,
    id: u32,
) -> RecordsResult<Player> {
    let r = sqlx::query_as("SELECT * FROM players WHERE id = ?")
        .bind(id)
        .fetch_one(db)
        .await?;
    Ok(r)
}

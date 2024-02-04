//! This module contains utility functions used to retrieve some objects from the database,
//! that must exist. This is why it is called `must`.
//!
//! This module is used when a route is called at a point where something must be already registered
//! in the database, like a player, a map, an event, etc.
//!
//! Unlike the Rust conventions, when such an object doesn't exist, the returned value isn't
//! `Option::None` but the corresponding error (for example, `RecordsError::PlayerNotFound`).
//! This makes the code cleaner thanks to the `?` syntax, because at some point we most likely want
//! things to be already existing, without checking it repeatedly and returning the error to
//! the client.

use sqlx::{Executor, MySql, MySqlConnection};

use crate::{
    error::{RecordsError, RecordsResult},
    event, map, models, player,
};

pub async fn have_event_handle(
    db: &mut MySqlConnection,
    handle: &str,
) -> RecordsResult<models::Event> {
    event::get_event_by_handle(db, handle)
        .await?
        .ok_or_else(|| RecordsError::EventNotFound(handle.to_owned()))
}

pub async fn have_event_edition(
    db: &mut MySqlConnection,
    event_handle: &str,
    edition_id: u32,
) -> RecordsResult<(models::Event, models::EventEdition)> {
    let event = have_event_handle(db, event_handle).await?;

    let Some(event_edition) = event::get_edition_by_id(db, event.id, edition_id).await? else {
        return Err(RecordsError::EventEditionNotFound(
            event_handle.to_owned(),
            edition_id,
        ));
    };

    Ok((event, event_edition))
}

pub async fn have_player<'c, E: sqlx::Executor<'c, Database = sqlx::MySql>>(
    db: E,
    login: &str,
) -> RecordsResult<models::Player> {
    player::get_player_from_login(db, login)
        .await?
        .ok_or_else(|| RecordsError::PlayerNotFound(login.to_owned()))
}

pub async fn have_map<'c, E: Executor<'c, Database = MySql>>(
    db: E,
    map_uid: &str,
) -> RecordsResult<models::Map> {
    map::get_map_from_game_id(db, map_uid)
        .await?
        .ok_or_else(|| RecordsError::MapNotFound(map_uid.to_owned()))
}

pub async fn have_event_edition_with_map(
    db: &mut MySqlConnection,
    game_id: &str,
    event_handle: String,
    edition_id: u32,
) -> RecordsResult<(models::Event, models::EventEdition)> {
    let (event, event_edition) = have_event_edition(db, &event_handle, edition_id).await?;

    if sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) AS map_in_event
        FROM event_edition_maps eem
        INNER JOIN maps m ON m.id = eem.map_id
        WHERE edition_id = ? AND event_id = ? AND game_id = ?",
    )
    .bind(event_edition.id)
    .bind(event.id)
    .bind(game_id)
    .fetch_one(db)
    .await?
        == 0
    {
        return Err(RecordsError::MapNotInEventEdition(
            game_id.to_owned(),
            event_handle,
            edition_id,
        ));
    }

    Ok((event, event_edition))
}

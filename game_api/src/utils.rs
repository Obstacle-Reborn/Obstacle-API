use actix_web::{HttpResponse, Responder};
use rand::Rng;
use records_lib::{models, Database};
use serde::Serialize;

use crate::{RecordsResult, RecordsResultExt};

/// Converts the provided body to a `200 OK` JSON responses.
pub fn json<T: Serialize, E>(obj: T) -> Result<impl Responder, E> {
    Ok(HttpResponse::Ok().json(obj))
}

/// Checks for any repeated item in a slice.
pub fn any_repeated<T: PartialEq>(slice: &[T]) -> bool {
    for (i, t) in slice.iter().enumerate() {
        if slice.split_at(i + 1).1.iter().any(|x| x == t) {
            return true;
        }
    }
    false
}

/// Returns a randomly-generated token with the `len` length. It contains alphanumeric characters.
pub fn generate_token(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .map(char::from)
        .take(len)
        .collect()
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ApiStatus {
    pub at: chrono::NaiveDateTime,
    #[sqlx(flatten)]
    pub kind: models::ApiStatusKind,
}

pub async fn get_api_status(db: &Database) -> RecordsResult<ApiStatus> {
    let result = sqlx::query_as(
        "SELECT a.*, sh1.status_history_date as `at`
        FROM api_status_history sh1
        INNER JOIN api_status a ON a.status_id = sh1.status_id
        WHERE sh1.status_history_id = (SELECT MAX(status_history_id) FROM api_status_history)",
    )
    .fetch_one(&db.mysql_pool)
    .await
    .with_api_err()?;

    Ok(result)
}

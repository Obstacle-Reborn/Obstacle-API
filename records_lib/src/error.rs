#[derive(thiserror::Error, Debug)]
#[repr(i32)]
pub enum RecordsError {
    // Caution: when creating a new error, you must ensure its code isn't
    // in conflict with another one in `game_api::RecordsErrorKind`.

    // --- Internal server errors
    #[error(transparent)]
    MySql(#[from] sqlx::Error) = 102,
    #[error(transparent)]
    Redis(#[from] deadpool_redis::redis::RedisError) = 103,
    #[error(transparent)]
    ExternalRequest(#[from] reqwest::Error) = 104,

    // --- Logical errors
    #[error("player not found in database: `{0}`")]
    PlayerNotFound(String) = 302,
    #[error("map not found in database")]
    MapNotFound(String) = 304,
    #[error("event `{0}` not found")]
    EventNotFound(String) = 310,
    #[error("event edition `{1}` not found for event `{0}`")]
    EventEditionNotFound(String, u32) = 311,
    #[error("map with uid `{0}` is not registered for event `{1}` edition {2}")]
    MapNotInEventEdition(String, String, u32) = 312,
}

impl RecordsError {
    pub fn get_code(&self) -> i32 {
        unsafe { *(self as *const Self as *const _) }
    }
}

pub type RecordsResult<T = ()> = Result<T, RecordsError>;

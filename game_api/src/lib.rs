use actix_web::dev::Payload;
use actix_web::{FromRequest, HttpRequest, HttpResponse};
use chrono::{DateTime, Utc};
use core::fmt;
use deadpool::managed::PoolError;
pub use deadpool_redis::Pool as RedisPool;
use records_lib::models;
use reqwest::Client;
use serde::{Deserialize, Serialize};
pub use sqlx::MySqlPool;
use std::convert::Infallible;
use std::fmt::Debug;
use std::future::{ready, Ready};
use std::io;
use std::ops::{Deref, DerefMut};
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;
use tracing_actix_web::RequestId;

mod auth;
mod graphql;
mod http;
pub(crate) mod must;
mod utils;

pub use auth::{get_tokens_ttl, AuthState};
pub use graphql::graphql_route;
pub use http::api_route;

#[derive(Deserialize, Debug, Clone)]
#[allow(unused)] // not used in code, but displayed as debug
pub struct AccessTokenErr {
    error: String,
    message: String,
    hint: String,
}

// TODO: make a field `Lib(records_lib::error::RecordsError)`
#[derive(Error, Debug)]
#[repr(i32)] // i32 to be used with clients that don't support unsigned integers
pub enum RecordsErrorKind {
    // Internal server errors
    #[error(transparent)]
    IOError(#[from] io::Error) = 101,
    #[error(transparent)]
    ExternalRequest(#[from] reqwest::Error) = 104,
    #[error("unknown error: {0}")]
    Unknown(String) = 105,
    #[error("server is in maintenance since {0}")]
    Maintenance(chrono::NaiveDateTime) = 106,
    #[error("unknown api status: `{0}` named `{1}`")]
    UnknownStatus(u8, String) = 107,

    // Authentication errors
    #[error("unauthorized")]
    Unauthorized = 201,
    #[error("forbidden")]
    Forbidden = 202,
    #[error("missing the /player/get_token request")]
    MissingGetTokenReq = 203,
    #[error("the state has already been received by the server")]
    StateAlreadyReceived(DateTime<Utc>) = 204,
    #[error("banned player {0}")]
    BannedPlayer(models::Banishment) = 205,
    #[error("error on sending request to MP services: {0:?}")]
    AccessTokenErr(AccessTokenErr) = 206,
    #[error("invalid ManiaPlanet code on /player/give_token request")]
    InvalidMPCode = 207,
    #[error("timeout exceeded (max 5 minutes)")]
    Timeout = 208,

    // Logical errors
    #[error("not found")]
    EndpointNotFound = 301,
    #[error("player not banned: `{0}`")]
    PlayerNotBanned(String) = 303,
    #[error("unknown role with id `{0}` and name `{1}`")]
    UnknownRole(u8, String) = 305,
    #[error("unknown medal with id `{0}` and name `{1}`")]
    UnknownMedal(u8, String) = 306,
    #[error("unknown rating kind with id `{0}` and name `{1}`")]
    UnknownRatingKind(u8, String) = 307,
    #[error("no rating found to update for player with login: `{0}` and map with uid: `{1}`")]
    NoRatingFound(String, String) = 308,
    #[error("invalid rates (too many, or repeated rate)")]
    InvalidRates = 309,
    #[error("invalid times")]
    InvalidTimes = 313,
    #[error("map pack id should be an integer, got `{0}`")]
    InvalidMappackId(String),

    #[error(transparent)]
    Lib(#[from] records_lib::error::RecordsError),
}

/// Converts a `Result<T, E>` in which `E` is convertible to [`records_lib::error::RecordsError`]
/// into a [`RecordsResult<T>`].
pub trait RecordsResultExt<T> {
    fn with_api_err(self) -> RecordsResult<T>;
}

impl<T, E> RecordsResultExt<T> for Result<T, E>
where
    records_lib::error::RecordsError: From<E>,
{
    fn with_api_err(self) -> RecordsResult<T> {
        self.map_err(records_lib::error::RecordsError::from)
            .map_err(Into::into)
    }
}

impl<E: Debug> From<PoolError<E>> for RecordsErrorKind {
    fn from(value: PoolError<E>) -> Self {
        Self::Unknown(format!("pool error: {value:?}"))
    }
}

impl<T> From<SendError<T>> for RecordsErrorKind {
    fn from(value: SendError<T>) -> Self {
        Self::Unknown(format!("send error: {value:?}"))
    }
}

#[derive(Debug)]
pub struct RecordsError {
    pub request_id: RequestId,
    pub kind: RecordsErrorKind,
}

impl fmt::Display for RecordsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} --- request id: {}", self.kind, self.request_id)
    }
}

impl std::error::Error for RecordsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.kind)
    }
}

impl RecordsError {
    fn to_err_res(&self) -> ErrorResponse {
        let message = self.to_string();
        ErrorResponse {
            request_id: self.request_id.to_string(),
            r#type: unsafe { *(&self.kind as *const RecordsErrorKind as *const _) },
            message,
        }
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    request_id: String,
    r#type: i32,
    message: String,
}

impl actix_web::ResponseError for RecordsError {
    fn error_response(&self) -> actix_web::HttpResponse {
        use records_lib::error::RecordsError as LR;
        use RecordsErrorKind as R;

        match &self.kind {
            // Internal server errors
            R::IOError(_) => HttpResponse::InternalServerError().json(self.to_err_res()),
            R::ExternalRequest(_) => HttpResponse::InternalServerError().json(self.to_err_res()),
            R::Unknown(_) => HttpResponse::InternalServerError().json(self.to_err_res()),
            R::Maintenance(_) => HttpResponse::InternalServerError().json(self.to_err_res()),
            R::UnknownStatus(..) => HttpResponse::InternalServerError().json(self.to_err_res()),

            // Authentication errors
            R::Unauthorized => HttpResponse::Unauthorized().json(self.to_err_res()),
            R::Forbidden => HttpResponse::Forbidden().json(self.to_err_res()),
            R::MissingGetTokenReq => HttpResponse::BadRequest().json(self.to_err_res()),
            R::StateAlreadyReceived(_) => HttpResponse::BadRequest().json(self.to_err_res()),
            R::BannedPlayer(ban) => {
                #[derive(Serialize)]
                struct BannedPlayerResponse<'a> {
                    message: String,
                    ban: &'a models::Banishment,
                }
                HttpResponse::Forbidden().json(BannedPlayerResponse {
                    message: ban.to_string(),
                    ban,
                })
            }
            R::AccessTokenErr(_) => HttpResponse::BadRequest().json(self.to_err_res()),
            R::InvalidMPCode => HttpResponse::BadRequest().json(self.to_err_res()),
            R::Timeout => HttpResponse::RequestTimeout().json(self.to_err_res()),

            // Logical errors
            R::EndpointNotFound => HttpResponse::NotFound().json(self.to_err_res()),
            R::PlayerNotBanned(_) => HttpResponse::BadRequest().json(self.to_err_res()),
            R::UnknownRole(..) => HttpResponse::InternalServerError().json(self.to_err_res()),
            R::UnknownMedal(..) => HttpResponse::InternalServerError().json(self.to_err_res()),
            R::UnknownRatingKind(..) => HttpResponse::InternalServerError().json(self.to_err_res()),
            R::NoRatingFound(..) => HttpResponse::BadRequest().json(self.to_err_res()),
            R::InvalidRates => HttpResponse::BadRequest().json(self.to_err_res()),
            R::InvalidTimes => HttpResponse::BadRequest().json(self.to_err_res()),
            R::InvalidMappackId(_) => HttpResponse::BadRequest().json(self.to_err_res()),

            R::Lib(e) => match e {
                // Internal server errors
                LR::MySql(_) => HttpResponse::InternalServerError().json(self.to_err_res()),
                LR::Redis(_) => HttpResponse::InternalServerError().json(self.to_err_res()),

                // Logical errors
                LR::PlayerNotFound(_) => HttpResponse::BadRequest().json(self.to_err_res()),
                LR::MapNotFound(_) => HttpResponse::BadRequest().json(self.to_err_res()),
                LR::EventNotFound(_) => HttpResponse::BadRequest().json(self.to_err_res()),
                LR::EventEditionNotFound(..) => HttpResponse::BadRequest().json(self.to_err_res()),
                LR::MapNotInEventEdition(..) => HttpResponse::BadRequest().json(self.to_err_res()),
            },
        }
    }
}

pub type RecordsResult<T> = Result<T, RecordsErrorKind>;

pub type RecordsResponse<T> = Result<T, RecordsError>;

pub trait FitRequestId<T, E> {
    fn fit(self, request_id: RequestId) -> RecordsResponse<T>;
}

impl<T, E> FitRequestId<T, E> for Result<T, E>
where
    RecordsErrorKind: From<E>,
{
    fn fit(self, request_id: RequestId) -> RecordsResponse<T> {
        self.map_err(|e| RecordsError {
            request_id,
            kind: e.into(),
        })
    }
}

/// Wrapper around [`reqwest::Client`] used to retrieve with the [`FromRequest`] trait.
///
/// We don't use [`Data`](actix_web::web::Data) because it wraps the object with an [`Arc`](std::sync::Arc),
/// and `reqwest::Client` already uses it.
pub struct ApiClient(Client);

impl Deref for ApiClient {
    type Target = Client;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ApiClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl FromRequest for ApiClient {
    type Error = Infallible;

    type Future = Ready<Result<Self, Infallible>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        let client = req
            .app_data::<Client>()
            .expect("Client should be present")
            .clone();
        ready(Ok(Self(client)))
    }
}

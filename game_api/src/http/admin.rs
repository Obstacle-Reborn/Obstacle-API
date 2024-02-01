use actix_web::{
    web::{self, Data, Json, Query},
    HttpResponse, Responder, Scope,
};
use records_lib::Database;
use serde::{Deserialize, Serialize};
use sqlx::{mysql::MySqlRow, FromRow, Row};
use tracing_actix_web::RequestId;

use crate::{
    auth::{privilege, MPAuthGuard},
    utils::json,
    FitRequestId, RecordsErrorKind, RecordsResponse, RecordsResult, RecordsResultExt,
};

pub fn admin_scope() -> Scope {
    web::scope("/admin")
        .route("/del_note", web::post().to(del_note))
        .route("/set_role", web::post().to(set_role))
        .route("/banishments", web::get().to(banishments))
        .route("/ban", web::post().to(ban))
        .route("/unban", web::post().to(unban))
        .route("/player_note", web::get().to(player_note))
}

#[derive(Deserialize)]
pub struct DelNoteBody {
    player_login: String,
}

pub async fn del_note(
    _: MPAuthGuard<{ privilege::ADMIN }>,
    req_id: RequestId,
    db: Data<Database>,
    Json(body): Json<DelNoteBody>,
) -> RecordsResponse<impl Responder> {
    sqlx::query("UPDATE players SET admins_note = NULL WHERE login = ?")
        .bind(&body.player_login)
        .execute(&db.mysql_pool)
        .await
        .with_api_err()
        .fit(req_id)?;

    Ok(HttpResponse::Ok().finish())
}

#[derive(Deserialize)]
pub struct SetRoleBody {
    player_login: String,
    role: u8,
}

#[derive(Serialize, Deserialize)]
struct SetRoleResponse {
    player_login: String,
    role: String,
}

pub async fn set_role(
    _: MPAuthGuard<{ privilege::ADMIN }>,
    req_id: RequestId,
    db: Data<Database>,
    Json(body): Json<SetRoleBody>,
) -> RecordsResponse<impl Responder> {
    sqlx::query("UPDATE players SET role = ? WHERE login = ?")
        .bind(body.role)
        .bind(&body.player_login)
        .execute(&db.mysql_pool)
        .await
        .with_api_err()
        .fit(req_id)?;

    let role = sqlx::query_scalar("SELECT role_name FROM role WHERE id = ?")
        .bind(body.role)
        .fetch_one(&db.mysql_pool)
        .await
        .with_api_err()
        .fit(req_id)?;

    json(SetRoleResponse {
        player_login: body.player_login,
        role,
    })
}

#[derive(Deserialize)]
pub struct BanishmentsBody {
    player_login: String,
}

#[derive(Serialize, FromRow)]
struct BanishmentInner {
    id: u32,
    date_ban: chrono::NaiveDateTime,
    duration: Option<u32>,
    reason: Option<String>,
    banished_by: String,
}

#[derive(Serialize)]
pub struct Banishment {
    #[serde(flatten)]
    inner: BanishmentInner,
    was_reprieved: bool,
    is_current: bool,
}

impl<'r> FromRow<'r, MySqlRow> for Banishment {
    fn from_row(row: &'r MySqlRow) -> Result<Self, sqlx::Error> {
        let is_current = match row.try_get::<i8, _>("is_current") {
            Ok(s) => s,
            Err(sqlx::Error::ColumnNotFound(_)) => 1,
            Err(e) => return Err(e),
        } != 0;

        Ok(Self {
            inner: BanishmentInner::from_row(row)?,
            was_reprieved: row.try_get::<i8, _>("was_reprieved")? != 0,
            is_current,
        })
    }
}

#[derive(Serialize)]
struct BanishmentsResponse {
    player_login: String,
    banishments: Vec<Banishment>,
}

pub async fn banishments(
    _: MPAuthGuard<{ privilege::ADMIN }>,
    req_id: RequestId,
    db: Data<Database>,
    Query(body): Query<BanishmentsBody>,
) -> RecordsResponse<impl Responder> {
    let player_id = records_lib::must::have_player(&db.mysql_pool, &body.player_login)
        .await
        .fit(req_id)?
        .id;

    let banishments = sqlx::query_as(
        "SELECT
            id, date_ban, duration, was_reprieved, reason,
            (SELECT login FROM players WHERE id = banished_by) AS banished_by,
            id IN (SELECT id FROM banishments WHERE player_id = ?
                AND (date_ban + INTERVAL duration SECOND > NOW() OR duration IS NULL)) AS is_current
        FROM banishments
        WHERE player_id = ?",
    )
    .bind(player_id)
    .bind(player_id)
    .fetch_all(&db.mysql_pool)
    .await
    .with_api_err()
    .fit(req_id)?;

    json(BanishmentsResponse {
        player_login: body.player_login,
        banishments,
    })
}

#[derive(Deserialize)]
pub struct BanBody {
    player_login: String,
    duration: Option<u32>,
    reason: Option<String>,
}

#[derive(Serialize)]
struct BanResponse {
    player_login: String,
    ban: Banishment,
}

pub async fn ban(
    MPAuthGuard { login }: MPAuthGuard<{ privilege::ADMIN }>,
    req_id: RequestId,
    db: Data<Database>,
    Json(body): Json<BanBody>,
) -> RecordsResponse<impl Responder> {
    let player_id = records_lib::must::have_player(&db.mysql_pool, &body.player_login)
        .await
        .fit(req_id)?
        .id;
    let admin_id = records_lib::must::have_player(&db.mysql_pool, &login)
        .await
        .fit(req_id)?
        .id;

    let was_reprieved =
        sqlx::query_as::<_, Banishment>("SELECT * FROM banishments WHERE player_id = ?")
            .bind(&body.player_login)
            .fetch_optional(&db.mysql_pool)
            .await
            .with_api_err()
            .fit(req_id)?
            .is_some();

    let ban_id: u32 = sqlx::query_scalar(
        "INSERT INTO banishments
                (date_ban,  duration,   was_reprieved,  reason, player_id,  banished_by)
        VALUES  (SYSDATE(), ?,          ?,              ?,      ?,          ?)
        RETURNING id",
    )
    .bind(body.duration)
    .bind(was_reprieved)
    .bind(body.reason)
    .bind(player_id)
    .bind(admin_id)
    .fetch_one(&db.mysql_pool)
    .await
    .with_api_err()
    .fit(req_id)?;

    let ban = sqlx::query_as(
        r#"SELECT id, date_ban, duration, reason, (SELECT login FROM players WHERE id = banished_by) as "banished_by", was_reprieved
        FROM banishments WHERE id = ?"#,
    )
    .bind(ban_id)
    .fetch_one(&db.mysql_pool)
    .await.with_api_err().fit(req_id)?;

    json(BanResponse {
        player_login: body.player_login,
        ban,
    })
}

pub async fn is_banned(db: &Database, player_id: u32) -> RecordsResult<Option<Banishment>> {
    let ban = sqlx::query_as(
        "SELECT id, date_ban, duration, was_reprieved, reason,
        (SELECT login FROM players WHERE id = banished_by) as banished_by
        FROM banishments
        WHERE player_id = ? AND (date_ban + INTERVAL duration SECOND > NOW() OR duration IS NULL)",
    )
    .bind(player_id)
    .fetch_optional(&db.mysql_pool)
    .await
    .with_api_err()?;

    Ok(ban)
}

#[derive(Deserialize)]
pub struct UnbanBody {
    player_login: String,
}

#[derive(Serialize)]
struct UnbanResponse {
    player_login: String,
    ban: Banishment,
}

pub async fn unban(
    _: MPAuthGuard<{ privilege::ADMIN }>,
    req_id: RequestId,
    db: Data<Database>,
    Json(body): Json<UnbanBody>,
) -> RecordsResponse<impl Responder> {
    let player_id = records_lib::must::have_player(&db.mysql_pool, &body.player_login)
        .await
        .fit(req_id)?
        .id;

    let Some(ban) = is_banned(&db, player_id).await.fit(req_id)? else {
        return Err(RecordsErrorKind::PlayerNotBanned(body.player_login)).fit(req_id);
    };

    if let Some(duration) =
        sqlx::query_scalar::<_, i64>("SELECT SYSDATE() - date_ban FROM banishments WHERE id = ?")
            .bind(ban.inner.id)
            .fetch_optional(&db.mysql_pool)
            .await
            .with_api_err()
            .fit(req_id)?
    {
        println!("duration: {duration}s");
    }

    sqlx::query("UPDATE banishments SET duration = SYSDATE() - date_ban WHERE id = ?")
        .bind(ban.inner.id)
        .execute(&db.mysql_pool)
        .await
        .with_api_err()
        .fit(req_id)?;

    json(UnbanResponse {
        player_login: body.player_login,
        ban,
    })
}

#[derive(Deserialize)]
pub struct PlayerNoteBody {
    player_login: String,
}

#[derive(Serialize)]
struct PlayerNoteResponse {
    player_login: String,
    admins_note: Option<String>,
}

pub async fn player_note(
    _: MPAuthGuard<{ privilege::ADMIN }>,
    req_id: RequestId,
    db: Data<Database>,
    Json(body): Json<PlayerNoteBody>,
) -> RecordsResponse<impl Responder> {
    let admins_note = records_lib::must::have_player(&db.mysql_pool, &body.player_login)
        .await
        .fit(req_id)?
        .admins_note;

    json(PlayerNoteResponse {
        player_login: body.player_login,
        admins_note,
    })
}

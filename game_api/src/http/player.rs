use std::sync::OnceLock;

use actix_session::Session;
use actix_web::{
    web::{self, Data, Json, Query},
    HttpResponse, Responder, Scope,
};
use records_lib::{get_env_var, models::Banishment, read_env_var_file, Database};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, MySqlPool};
use tokio::time::timeout;
use tracing::Level;
use tracing_actix_web::RequestId;

use crate::{
    auth::{
        self, privilege, ApiAvailable, AuthHeader, AuthState, MPAuthGuard, Message, WebToken,
        TIMEOUT, WEB_TOKEN_SESS_KEY,
    },
    utils::json,
    AccessTokenErr, ApiClient, FitRequestId, RecordsErrorKind, RecordsResponse, RecordsResult,
    RecordsResultExt,
};

use super::{admin, pb, player_finished as pf};

pub fn player_scope() -> Scope {
    web::scope("/player")
        .route("/update", web::post().to(update))
        .route("/finished", web::post().to(finished))
        .route("/get_token", web::post().to(get_token))
        .route("/give_token", web::post().to(post_give_token))
        .route("/pb", web::get().to(pb))
        .route("/times", web::post().to(times))
        .route("/info", web::get().to(info))
        .route("/report_error", web::post().to(report_error))
}

#[derive(Serialize, Deserialize, Clone, FromRow, Debug)]
pub struct UpdatePlayerBody {
    pub login: String,
    pub name: String,
    pub zone_path: Option<String>,
}

async fn insert_player(db: &Database, body: &UpdatePlayerBody) -> RecordsResult<u32> {
    let id = sqlx::query_scalar(
        "INSERT INTO players
        (login, name, join_date, zone_path, admins_note, role)
        VALUES (?, ?, SYSDATE(), ?, NULL, 0) RETURNING id",
    )
    .bind(&body.login)
    .bind(&body.name)
    .bind(&body.zone_path)
    .fetch_one(&db.mysql_pool)
    .await
    .with_api_err()?;

    Ok(id)
}

pub async fn get_or_insert(db: &Database, body: &UpdatePlayerBody) -> RecordsResult<u32> {
    if let Some(id) = sqlx::query_scalar("SELECT id FROM players WHERE login = ?")
        .bind(&body.login)
        .fetch_optional(&db.mysql_pool)
        .await
        .with_api_err()?
    {
        return Ok(id);
    }

    insert_player(db, body).await
}

pub async fn update(
    _: ApiAvailable,
    req_id: RequestId,
    db: Data<Database>,
    AuthHeader { login, token }: AuthHeader,
    Json(body): Json<UpdatePlayerBody>,
) -> RecordsResponse<impl Responder> {
    match auth::check_auth_for(&db, &login, &token, privilege::PLAYER).await {
        Ok(id) => update_player(&db, id, body).await.fit(req_id)?,
        // At this point, if Redis has registered a token with the login, it means that
        // the player is not yet added to the Obstacle database but effectively
        // has a ManiaPlanet account
        Err(RecordsErrorKind::Lib(records_lib::error::RecordsError::PlayerNotFound(_))) => {
            let _ = insert_player(&db, &body).await.fit(req_id)?;
        }
        Err(e) => return Err(e).fit(req_id),
    }

    Ok(HttpResponse::Ok().finish())
}

pub async fn update_player(
    db: &Database,
    player_id: u32,
    body: UpdatePlayerBody,
) -> RecordsResult<()> {
    sqlx::query("UPDATE players SET name = ?, zone_path = ? WHERE id = ?")
        .bind(body.name)
        .bind(body.zone_path)
        .bind(player_id)
        .execute(&db.mysql_pool)
        .await
        .with_api_err()?;

    Ok(())
}

pub async fn check_banned(
    db: &MySqlPool,
    player_id: u32,
) -> Result<Option<Banishment>, RecordsErrorKind> {
    let r = sqlx::query_as("SELECT * FROM current_bans WHERE player_id = ?")
        .bind(player_id)
        .fetch_optional(db)
        .await
        .with_api_err()?;
    Ok(r)
}

#[derive(Serialize)]
struct MPAccessTokenBody<'a> {
    grant_type: &'a str,
    client_id: &'a str,
    client_secret: &'a str,
    code: &'a str,
    redirect_uri: &'a str,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MPAccessTokenResponse {
    AccessToken { access_token: String },
    Error(AccessTokenErr),
}

#[derive(Deserialize, Debug)]
struct MPServerRes {
    #[serde(alias = "login")]
    res_login: String,
}

static MP_APP_CLIENT_ID: OnceLock<String> = OnceLock::new();
static MP_APP_CLIENT_SECRET: OnceLock<String> = OnceLock::new();

fn get_mp_app_client_id() -> &'static str {
    MP_APP_CLIENT_ID.get_or_init(|| read_env_var_file("RECORDS_MP_APP_CLIENT_ID_FILE"))
}

fn get_mp_app_client_secret() -> &'static str {
    MP_APP_CLIENT_SECRET.get_or_init(|| read_env_var_file("RECORDS_MP_APP_CLIENT_SECRET_FILE"))
}

async fn test_access_token(
    client: &Client,
    login: &str,
    code: &str,
    redirect_uri: &str,
) -> RecordsResult<bool> {
    let res = client
        .post("https://prod.live.maniaplanet.com/login/oauth2/access_token")
        .form(&MPAccessTokenBody {
            grant_type: "authorization_code",
            client_id: get_mp_app_client_id(),
            client_secret: get_mp_app_client_secret(),
            code,
            redirect_uri,
        })
        .send()
        .await?
        .json()
        .await?;

    let access_token = match res {
        MPAccessTokenResponse::AccessToken { access_token } => access_token,
        MPAccessTokenResponse::Error(err) => return Err(RecordsErrorKind::AccessTokenErr(err)),
    };

    check_mp_token(client, login, access_token).await
}

async fn check_mp_token(client: &Client, login: &str, token: String) -> RecordsResult<bool> {
    let res = client
        .get("https://prod.live.maniaplanet.com/webservices/me")
        .header("Accept", "application/json")
        .bearer_auth(token)
        .send()
        .await?;
    let MPServerRes { res_login } = match res.status() {
        StatusCode::OK => res.json().await?,
        _ => return Ok(false),
    };

    Ok(res_login.to_lowercase() == login.to_lowercase())
}

async fn finished(
    _: ApiAvailable,
    req_id: RequestId,
    MPAuthGuard { login }: MPAuthGuard<{ privilege::PLAYER }>,
    db: Data<Database>,
    body: pf::PlayerFinishedBody,
) -> RecordsResponse<impl Responder> {
    let res = pf::finished(login, &db, body, None).await.fit(req_id)?.res;
    json(res)
}

#[derive(Deserialize)]
pub struct GetTokenBody {
    login: String,
    state: String,
    redirect_uri: String,
}

#[derive(Serialize)]
struct GetTokenResponse {
    token: String,
}

pub async fn get_token(
    _: ApiAvailable,
    req_id: RequestId,
    db: Data<Database>,
    ApiClient(client): ApiClient,
    state: Data<AuthState>,
    Json(body): Json<GetTokenBody>,
) -> RecordsResponse<impl Responder> {
    // retrieve access_token from browser redirection
    let (tx, rx) = state
        .connect_with_browser(body.state.clone())
        .await
        .fit(req_id)?;
    let code = match timeout(TIMEOUT, rx).await {
        Ok(Ok(Message::MPCode(access_token))) => access_token,
        _ => {
            tracing::event!(
                Level::WARN,
                "Token state `{}` timed out, removing it",
                body.state.clone()
            );
            state.remove_state(body.state).await;
            return Err(RecordsErrorKind::Timeout).fit(req_id);
        }
    };

    let err_msg = "/get_token rx should not be dropped at this point";

    // check access_token and generate new token for player ...
    match test_access_token(&client, &body.login, &code, &body.redirect_uri).await {
        Ok(true) => (),
        Ok(false) => {
            tx.send(Message::InvalidMPCode).expect(err_msg);
            return Err(RecordsErrorKind::InvalidMPCode).fit(req_id);
        }
        Err(RecordsErrorKind::AccessTokenErr(err)) => {
            tx.send(Message::AccessTokenErr(err.clone()))
                .expect(err_msg);
            return Err(RecordsErrorKind::AccessTokenErr(err)).fit(req_id);
        }
        err => {
            let _ = err.fit(req_id)?;
        }
    }

    let (mp_token, web_token) = auth::gen_token_for(&db, &body.login).await.fit(req_id)?;
    tx.send(Message::Ok(WebToken {
        login: body.login,
        token: web_token,
    }))
    .expect(err_msg);

    json(GetTokenResponse { token: mp_token })
}

#[derive(Deserialize)]
pub struct GiveTokenBody {
    code: String,
    state: String,
}

#[derive(Serialize)]
pub struct GiveTokenResponse {
    login: String,
    token: String,
}

pub async fn post_give_token(
    req_id: RequestId,
    session: Session,
    state: Data<AuthState>,
    Json(body): Json<GiveTokenBody>,
) -> RecordsResponse<impl Responder> {
    let web_token = state
        .browser_connected_for(body.state, body.code)
        .await
        .fit(req_id)?;
    session
        .insert(WEB_TOKEN_SESS_KEY, web_token)
        .expect("unable to insert session web token");
    Ok(HttpResponse::Ok().finish())
}

#[derive(Serialize)]
struct IsBannedResponse {
    login: String,
    banned: bool,
    current_ban: Option<admin::Banishment>,
}

async fn pb(
    _: ApiAvailable,
    req_id: RequestId,
    MPAuthGuard { login }: MPAuthGuard<{ privilege::PLAYER }>,
    db: Data<Database>,
    body: pb::PbReq,
) -> RecordsResponse<impl Responder> {
    pb::pb(login, req_id, db, body, None).await
}

#[derive(Deserialize)]
struct TimesBody {
    maps_uids: Vec<String>,
}

#[derive(Serialize, FromRow)]
struct TimesResponseItem {
    map_uid: String,
    time: i32,
}

async fn times(
    req_id: RequestId,
    MPAuthGuard { login }: MPAuthGuard<{ privilege::PLAYER }>,
    db: Data<Database>,
    Json(body): Json<TimesBody>,
) -> RecordsResponse<impl Responder> {
    let player = records_lib::must::have_player(&db.mysql_pool, &login)
        .await
        .fit(req_id)?;

    let query = format!(
        "SELECT m.game_id AS map_uid, MIN(r.time) AS time
        FROM maps m
        INNER JOIN records r ON r.map_id = m.id
        WHERE r.record_player_id = ? AND m.game_id IN ({})
        GROUP BY m.id",
        body.maps_uids
            .iter()
            .map(|_| "?".to_owned())
            .collect::<Vec<_>>()
            .join(",")
    );

    let mut query = sqlx::query_as::<_, TimesResponseItem>(&query).bind(player.id);

    for map_uid in body.maps_uids {
        query = query.bind(map_uid);
    }

    let result = query
        .fetch_all(&db.mysql_pool)
        .await
        .with_api_err()
        .fit(req_id)?;
    json(result)
}

#[derive(Deserialize)]
pub struct InfoBody {
    login: String,
}

#[derive(Serialize, FromRow)]
struct InfoResponse {
    id: u32,
    login: String,
    name: String,
    join_date: Option<chrono::NaiveDateTime>,
    zone_path: Option<String>,
    role_name: String,
}

pub async fn info(
    req_id: RequestId,
    db: Data<Database>,
    Query(body): Query<InfoBody>,
) -> RecordsResponse<impl Responder> {
    let Some(info) = sqlx::query_as::<_, InfoResponse>(
        "SELECT *, (SELECT role_name FROM role WHERE id = role) as role_name
        FROM players WHERE login = ?",
    )
    .bind(&body.login)
    .fetch_optional(&db.mysql_pool)
    .await
    .with_api_err()
    .fit(req_id)?
    else {
        return Err(RecordsErrorKind::from(
            records_lib::error::RecordsError::PlayerNotFound(body.login),
        ))
        .fit(req_id);
    };

    json(info)
}

#[derive(Deserialize)]
struct ReportErrorBody {
    on_route: String,
    request_id: String,
    map_uid: String,
    err_type: i32,
    err_msg: String,
    time: i32,
    respawn_count: i32,
}

async fn report_error(
    req_id: RequestId,
    MPAuthGuard { login }: MPAuthGuard<{ privilege::PLAYER }>,
    ApiClient(client): ApiClient,
    Json(body): Json<ReportErrorBody>,
) -> RecordsResponse<impl Responder> {
    #[derive(Serialize)]
    struct WebhookBodyEmbedField {
        name: String,
        value: String,
    }

    #[derive(Serialize)]
    struct WebhookBodyEmbed {
        title: String,
        description: Option<String>,
        color: u32,
        fields: Option<Vec<WebhookBodyEmbedField>>,
    }

    #[derive(Serialize)]
    struct WebhookBody {
        content: String,
        embeds: Vec<WebhookBodyEmbed>,
    }

    let url = get_env_var("WEBHOOK_REPORT_URL");

    let mut fields = vec![
        WebhookBodyEmbedField {
            name: "Map UID".to_owned(),
            value: format!("`{}`", body.map_uid),
        },
        WebhookBodyEmbedField {
            name: "When called this API route".to_owned(),
            value: format!("`{}`", body.on_route),
        },
        WebhookBodyEmbedField {
            name: "Request ID".to_owned(),
            value: format!("`{}`", body.request_id),
        },
    ];

    let (content, color) = if body.on_route == "/player/finished" {
        fields.extend(
            vec![
                WebhookBodyEmbedField {
                    name: "Run time".to_owned(),
                    value: format!("`{}`", body.time),
                },
                WebhookBodyEmbedField {
                    name: "Respawn count".to_owned(),
                    value: format!("`{}`", body.respawn_count),
                },
            ]
            .into_iter(),
        );

        (
            format!("🚨 Player `{login}` finished a map but got an error."),
            11862016,
        )
    } else {
        (
            format!("⚠️ Player `{login}` got an error while playing."),
            5814783,
        )
    };

    client
        .post(url)
        .json(&WebhookBody {
            content,
            embeds: vec![
                WebhookBodyEmbed {
                    title: format!("Error type {}", body.err_type),
                    description: Some(format!("`{}`", body.err_msg)),
                    color,
                    fields: None,
                },
                WebhookBodyEmbed {
                    title: "Context".to_owned(),
                    description: None,
                    color,
                    fields: Some(fields),
                },
            ],
        })
        .send()
        .await
        .fit(req_id)?;

    Ok(HttpResponse::Ok().finish())
}

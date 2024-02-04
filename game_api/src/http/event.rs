use actix_web::{
    web::{self, Data, Path},
    Responder, Scope,
};
use itertools::Itertools;
use records_lib::{models, Database};
use serde::Serialize;
use sqlx::FromRow;
use tracing_actix_web::RequestId;

use crate::{
    auth::{privilege, MPAuthGuard},
    utils::json,
    FitRequestId, RecordsResponse, RecordsResult, RecordsResultExt,
};

use super::{overview, pb, player::UpdatePlayerBody, player_finished as pf};

pub fn event_scope() -> Scope {
    web::scope("/event")
        .service(
            web::scope("/{event_handle}")
                .service(
                    web::scope("/{edition_id}")
                        .route("/overview", web::get().to(edition_overview))
                        .service(
                            web::scope("/player")
                                .route("/finished", web::post().to(edition_finished))
                                .route("/pb", web::get().to(edition_pb)),
                        )
                        .default_service(web::get().to(edition)),
                )
                .default_service(web::get().to(event_editions)),
        )
        .default_service(web::get().to(event_list))
}

pub async fn get_categories_by_edition_id(
    db: &Database,
    event_id: u32,
    edition_id: u32,
) -> RecordsResult<Vec<models::EventCategory>> {
    let r = sqlx::query_as(
        "SELECT DISTINCT ec.* FROM event_edition ee
        LEFT JOIN event_edition_categories eec ON eec.edition_id = ee.id
        LEFT JOIN event_categories ecs ON ecs.event_id = ee.event_id
        INNER JOIN event_category ec ON ec.id IN (eec.category_id, ecs.category_id)
        WHERE ee.event_id = ? AND ee.id = ?
        ORDER BY ecs.category_id DESC",
    )
    .bind(event_id)
    .bind(edition_id)
    .fetch_all(&db.mysql_pool)
    .await
    .with_api_err()?;

    Ok(r)
}

#[derive(FromRow)]
struct MapWithCategory {
    #[sqlx(flatten)]
    map: models::Map,
    category_id: Option<u32>,
    mx_id: i64,
}

async fn get_maps_by_edition_id(
    db: &Database,
    event_id: u32,
    edition_id: u32,
) -> RecordsResult<Vec<MapWithCategory>> {
    let r = sqlx::query_as(
        "SELECT m.*, category_id, mx_id
        FROM maps m
        INNER JOIN event_edition_maps eem ON id = map_id
        AND event_id = ? AND edition_id = ?
        ORDER BY category_id, eem.order",
    )
    .bind(event_id)
    .bind(edition_id)
    .fetch_all(&db.mysql_pool)
    .await
    .with_api_err()?;

    Ok(r)
}

#[derive(Serialize, FromRow)]
pub struct EventResponse {
    handle: String,
    last_edition_id: i64,
}

#[derive(Serialize, FromRow)]
struct EventHandleResponse {
    id: u32,
    name: String,
    start_date: chrono::NaiveDateTime,
}

#[derive(Serialize)]
struct Map {
    mx_id: i64,
    main_author: UpdatePlayerBody,
    name: String,
    map_uid: String,
    bronze_time: i32,
    silver_time: i32,
    gold_time: i32,
    champion_time: i32,
    personal_best: i32,
}

#[derive(Serialize, Default)]
struct Category {
    handle: String,
    name: String,
    banner_img_url: String,
    maps: Vec<Map>,
}

impl From<Vec<Map>> for Category {
    fn from(maps: Vec<Map>) -> Self {
        Self {
            maps,
            ..Default::default()
        }
    }
}

#[derive(Serialize)]
struct EventHandleEditionResponse {
    id: u32,
    name: String,
    start_date: chrono::NaiveDateTime,
    banner_img_url: String,
    mx_id: i32,
    categories: Vec<Category>,
}

async fn event_list(req_id: RequestId, db: Data<Database>) -> RecordsResponse<impl Responder> {
    let out = sqlx::query_as::<_, EventResponse>(
        "WITH handles AS (SELECT ev.handle AS handle, MAX(ed.id) AS last_id
        FROM event ev
        LEFT JOIN event_edition ed ON ed.event_id = ev.id
        GROUP BY ev.id, ev.handle
        ORDER BY ev.id DESC)
        SELECT handle, CAST(IF(last_id IS NULL, -1, last_id) AS INT) AS last_edition_id
        FROM handles",
    )
    .fetch_all(&db.mysql_pool)
    .await
    .with_api_err()
    .fit(req_id)?;

    json(out)
}

async fn event_editions(
    db: Data<Database>,
    req_id: RequestId,
    event_handle: Path<String>,
) -> RecordsResponse<impl Responder> {
    let event_handle = event_handle.into_inner();

    let mysql_conn = &mut db.mysql_pool.acquire().await.with_api_err().fit(req_id)?;

    let id = records_lib::must::have_event_handle(mysql_conn, &event_handle)
        .await
        .fit(req_id)?
        .id;

    let res: Vec<EventHandleResponse> =
        sqlx::query_as("SELECT * FROM event_edition WHERE event_id = ? ORDER BY id DESC")
            .bind(id)
            .fetch_all(&mut **mysql_conn)
            .await
            .with_api_err()
            .fit(req_id)?;

    json(res)
}

#[derive(FromRow)]
struct AuthorWithPlayerTime {
    /// The author of the map
    #[sqlx(flatten)]
    main_author: UpdatePlayerBody,
    // The time of the player (not the same player as the author)
    personal_best: i32,
}

async fn edition(
    auth: Option<MPAuthGuard<{ privilege::PLAYER }>>,
    db: Data<Database>,
    req_id: RequestId,
    path: Path<(String, u32)>,
) -> RecordsResponse<impl Responder> {
    let (event_handle, edition_id) = path.into_inner();

    let mysql_conn = &mut db.mysql_pool.acquire().await.with_api_err().fit(req_id)?;

    let (models::Event { id: event_id, .. }, edition) =
        records_lib::must::have_event_edition(mysql_conn, &event_handle, edition_id)
            .await
            .fit(req_id)?;

    let maps = get_maps_by_edition_id(&db, event_id, edition_id)
        .await
        .fit(req_id)?
        .into_iter()
        .group_by(|m| m.category_id);
    let maps = maps.into_iter();

    let mut cat = get_categories_by_edition_id(&db, event_id, edition.id)
        .await
        .fit(req_id)?;

    let mut categories = Vec::with_capacity(cat.len());

    for (cat_id, cat_maps) in maps {
        let m = cat_id
            .and_then(|c_id| cat.iter().find_position(|c| c.id == c_id))
            .map(|(i, _)| i)
            .map(|i| cat.swap_remove(i))
            .unwrap_or_default();

        let mut maps = Vec::with_capacity(cat_maps.size_hint().0);

        for MapWithCategory { map, mx_id, .. } in cat_maps {
            let AuthorWithPlayerTime {
                main_author,
                personal_best,
            } = if let Some(MPAuthGuard { login }) = &auth {
                let main_author = sqlx::query_as("select * from players where id = ?")
                    .bind(map.player_id)
                    .fetch_one(&db.mysql_pool)
                    .await
                    .with_api_err()
                    .fit(req_id)?;

                let personal_best = sqlx::query_scalar(
                    "select time from global_records r
                    inner join players p on p.id = r.record_player_id
                    inner join event_edition_records eer on eer.record_id = r.record_id
                        and eer.event_id = ? and eer.edition_id = ?
                    where p.login = ? and r.map_id = ?",
                )
                .bind(event_id)
                .bind(edition_id)
                .bind(login)
                .bind(map.id)
                .fetch_optional(&db.mysql_pool)
                .await
                .with_api_err()
                .fit(req_id)?
                .unwrap_or(-1);

                Ok(AuthorWithPlayerTime {
                    main_author,
                    personal_best,
                })
            } else {
                sqlx::query_as("select a.*, -1 as personal_best from players a where a.id = ?")
                    .bind(map.player_id)
                    .fetch_one(&db.mysql_pool)
                    .await
            }
            .with_api_err()
            .fit(req_id)?;

            let (bronze_time, silver_time, gold_time, champion_time) = sqlx::query_as("
                select bronze.time, silver.time, gold.time, champion.time
                from event_edition_maps_medals bronze, event_edition_maps_medals silver, event_edition_maps_medals gold, event_edition_maps_medals champion
                where bronze.event_id = silver.event_id and silver.event_id = gold.event_id and gold.event_id = champion.event_id
                    and bronze.edition_id = silver.edition_id and silver.edition_id = gold.edition_id and gold.edition_id = champion.edition_id
                    and bronze.map_id = silver.map_id and silver.map_id = gold.map_id and gold.map_id = champion.map_id
                    and bronze.medal_id = 1 and silver.medal_id = 2 and gold.medal_id = 3 and champion.medal_id = 4
                    and bronze.map_id = ? and bronze.event_id = ? and bronze.edition_id = ?")
            .bind(map.id).bind(event_id).bind(edition_id).fetch_optional(&db.mysql_pool).await.with_api_err().fit(req_id)?.unwrap_or((-1, -1, -1, -1));

            maps.push(Map {
                mx_id,
                main_author,
                name: map.name,
                map_uid: map.game_id,
                bronze_time,
                silver_time,
                gold_time,
                champion_time,
                personal_best,
            });
        }

        categories.push(Category {
            handle: m.handle,
            name: m.name,
            banner_img_url: m.banner_img_url.unwrap_or_default(),
            maps,
        });
    }

    // Fill with empty categories
    for cat in cat {
        categories.push(Category {
            handle: cat.handle,
            name: cat.name,
            banner_img_url: cat.banner_img_url.unwrap_or_default(),
            maps: Vec::new(),
        });
    }

    json(EventHandleEditionResponse {
        id: edition.id,
        name: edition.name,
        start_date: edition.start_date,
        banner_img_url: edition.banner_img_url.unwrap_or_default(),
        mx_id: edition.mx_id.unwrap_or(-1),
        categories,
    })
}

async fn edition_overview(
    req_id: RequestId,
    db: Data<Database>,
    path: Path<(String, u32)>,
    query: overview::OverviewReq,
) -> RecordsResponse<impl Responder> {
    overview::overview(req_id, db, query, Some(path.into_inner())).await
}

async fn edition_finished(
    MPAuthGuard { login }: MPAuthGuard<{ privilege::PLAYER }>,
    req_id: RequestId,
    db: Data<Database>,
    path: Path<(String, u32)>,
    body: pf::PlayerFinishedBody,
) -> RecordsResponse<impl Responder> {
    let (event_handle, edition_id) = path.into_inner();

    let mysql_conn = &mut db.mysql_pool.acquire().await.with_api_err().fit(req_id)?;

    // We first check that the event and its edition exist
    // and that the map is registered on it.
    let event = records_lib::must::have_event_edition_with_map(
        mysql_conn,
        &body.map_uid,
        event_handle,
        edition_id,
    )
    .await
    .fit(req_id)?;

    // Then we insert the record for the global records
    let res = pf::finished(login, &db, body, Some(&event))
        .await
        .fit(req_id)?;

    // Then we insert it for the event edition records.
    // This is not part of the transaction, because we don't want to rollback
    // the insertion of the record if this query fails.
    let (event, edition) = event;
    sqlx::query(
        "INSERT INTO event_edition_records (record_id, event_id, edition_id)
            VALUES (?, ?, ?)",
    )
    .bind(res.record_id)
    .bind(event.id)
    .bind(edition.id)
    .execute(&db.mysql_pool)
    .await
    .with_api_err()
    .fit(req_id)?;

    json(res.res)
}

async fn edition_pb(
    MPAuthGuard { login }: MPAuthGuard<{ privilege::PLAYER }>,
    req_id: RequestId,
    path: Path<(String, u32)>,
    db: Data<Database>,
    body: pb::PbReq,
) -> RecordsResponse<impl Responder> {
    let (event_handle, edition_id) = path.into_inner();

    let mysql_conn = &mut db.mysql_pool.acquire().await.with_api_err().fit(req_id)?;

    let event = records_lib::must::have_event_edition_with_map(
        mysql_conn,
        &body.map_uid,
        event_handle,
        edition_id,
    )
    .await
    .fit(req_id)?;
    pb::pb(login, req_id, db, body, Some(event)).await
}

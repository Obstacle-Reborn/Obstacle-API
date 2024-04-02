use actix_web::{
    web::{Data, Query},
    Responder,
};
use futures::StreamExt;
use records_lib::{models, Database, GetSqlFragments};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tracing_actix_web::RequestId;

use crate::{utils::json, FitRequestId, RecordsResponse, RecordsResultExt};

#[derive(Deserialize)]
pub struct PbBody {
    pub map_uid: String,
}

pub type PbReq = Query<PbBody>;

#[derive(Serialize)]
struct PbResponse {
    rs_count: i32,
    cps_times: Vec<PbCpTimesResponseItem>,
}

#[derive(FromRow)]
struct PbResponseItem {
    rs_count: i32,
    cp_num: u32,
    time: i32,
}

#[derive(Serialize, FromRow)]
struct PbCpTimesResponseItem {
    cp_num: u32,
    time: i32,
}

pub async fn pb(
    login: String,
    req_id: RequestId,
    db: Data<Database>,
    Query(PbBody { map_uid }): PbReq,
    event: Option<(&models::Event, &models::EventEdition)>,
) -> RecordsResponse<impl Responder> {
    let (join_event, and_event) = event.get_sql_fragments();

    let query = format!(
        "SELECT r.respawn_count AS rs_count, cps.cp_num AS cp_num, cps.time AS time
        FROM checkpoint_times cps
        INNER JOIN maps m ON m.id = cps.map_id
        INNER JOIN records r ON r.record_id = cps.record_id
        {join_event}
        INNER JOIN players p on r.record_player_id = p.id
        WHERE m.game_id = ? AND p.login = ?
            {and_event}
            AND r.time = (
                SELECT IF(m.reversed, MAX(time), MIN(time))
                FROM records r
                {join_event}
                WHERE r.map_id = m.id AND p.id = r.record_player_id
                    {and_event}
            )"
    );

    let mut query = sqlx::query_as::<_, PbResponseItem>(&query)
        .bind(map_uid)
        .bind(login);

    if let Some((event, edition)) = event {
        query = query
            .bind(event.id)
            .bind(edition.id)
            .bind(event.id)
            .bind(edition.id);
    }

    let mut times = query.fetch(&db.mysql_pool);

    let mut res = PbResponse {
        rs_count: 0,
        cps_times: Vec::with_capacity(times.size_hint().0),
    };

    while let Some(PbResponseItem {
        rs_count,
        cp_num,
        time,
    }) = times.next().await.transpose().with_api_err().fit(req_id)?
    {
        res.rs_count = rs_count;
        res.cps_times.push(PbCpTimesResponseItem { cp_num, time });
    }

    json(res)
}

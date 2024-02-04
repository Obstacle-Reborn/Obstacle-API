use async_graphql::SimpleObject;
use deadpool_redis::{redis::AsyncCommands, Connection as RedisConnection};
use records_lib::{
    must, player,
    redis_key::{
        mappack_key, mappack_lb_key, mappack_map_last_rank, mappack_nb_map_key,
        mappack_player_map_finished_key, mappack_player_rank_avg_key, mappack_player_ranks_key,
        mappack_player_worst_rank_key, MappackKey,
    },
    update_mappacks::update_mappack,
    Database, MySqlPool, RedisPool,
};
use reqwest::Client;
use serde::Deserialize;
use sqlx::MySqlConnection;

use crate::{RecordsErrorKind, RecordsResult, RecordsResultExt};

use super::{map::Map, player::Player};

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct MXMappackResponseItem {
    TrackUID: String,
}

async fn fill_mappack(
    client: &Client,
    db: &mut MySqlConnection,
    redis_conn: &mut RedisConnection,
    mappack_key: &MappackKey<'_>,
    mappack_id: u32,
) -> RecordsResult<()> {
    let res: Vec<MXMappackResponseItem> = client
        .get(format!(
            "https://sm.mania.exchange/api/mappack/get_mappack_tracks/{mappack_id}"
        ))
        .header("User-Agent", "obstacle (ahmadbky@5382)")
        .send()
        .await?
        .json()
        .await?;

    for MXMappackResponseItem { TrackUID } in res {
        // We check that the map exists in our database
        let _ = must::have_map(&mut *db, &TrackUID).await?;
        redis_conn
            .sadd(mappack_key, TrackUID)
            .await
            .with_api_err()?;
    }

    Ok(())
}

pub struct Mappack {
    pub(crate) mappack_id: String,
}

#[derive(SimpleObject)]
struct MappackMap {
    rank: i32,
    last_rank: i32,
    map: Map,
}

struct MappackPlayer<'a> {
    mappack_id: &'a str,
    inner: Player,
}

#[async_graphql::Object]
impl MappackPlayer<'_> {
    async fn rank(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<usize> {
        let redis_pool = ctx.data_unchecked::<RedisPool>();
        let redis_conn = &mut redis_pool.get().await?;
        let rank = redis_conn
            .zscore(mappack_lb_key(self.mappack_id), self.inner.inner.id)
            .await?;
        Ok(rank)
    }

    async fn player(&self) -> &Player {
        &self.inner
    }

    async fn ranks(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Vec<MappackMap>> {
        let redis_pool = ctx.data_unchecked::<RedisPool>();
        let mysql_pool = ctx.data_unchecked::<MySqlPool>();

        let redis_conn = &mut redis_pool.get().await?;
        let mysql_conn = &mut mysql_pool.acquire().await?;

        let maps_uids: Vec<String> = redis_conn
            .zrange_withscores(
                mappack_player_ranks_key(self.mappack_id, self.inner.inner.id),
                0,
                -1,
            )
            .await?;
        let maps_uids = maps_uids.chunks_exact(2);

        let mut out = Vec::with_capacity(maps_uids.size_hint().0);

        for chunk in maps_uids {
            let [game_id, rank] = chunk else {
                unreachable!("plz stabilize Iterator::array_chunks")
            };
            let rank = rank.parse()?;
            let last_rank = redis_conn
                .get(mappack_map_last_rank(self.mappack_id, game_id))
                .await?;
            let map = must::have_map(&mut **mysql_conn, game_id).await?;

            out.push(MappackMap {
                map: map.into(),
                rank,
                last_rank,
            });
        }

        Ok(out)
    }

    async fn rank_avg(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<f64> {
        let redis_pool = ctx.data_unchecked::<RedisPool>();
        let redis_conn = &mut redis_pool.get().await?;
        let rank = redis_conn
            .get(mappack_player_rank_avg_key(
                self.mappack_id,
                self.inner.inner.id,
            ))
            .await?;
        Ok(rank)
    }

    async fn map_finished(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<usize> {
        let redis_pool = ctx.data_unchecked::<RedisPool>();
        let redis_conn = &mut redis_pool.get().await?;
        let map_finished = redis_conn
            .get(mappack_player_map_finished_key(
                self.mappack_id,
                self.inner.inner.id,
            ))
            .await?;
        Ok(map_finished)
    }

    async fn worst_rank(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<i32> {
        let redis_pool = ctx.data_unchecked::<RedisPool>();
        let redis_conn = &mut redis_pool.get().await?;
        let worst_rank = redis_conn
            .get(mappack_player_worst_rank_key(
                self.mappack_id,
                self.inner.inner.id,
            ))
            .await?;
        Ok(worst_rank)
    }
}

#[async_graphql::Object]
impl Mappack {
    async fn nb_maps(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<usize> {
        let redis_pool = ctx.data_unchecked::<RedisPool>();
        let redis_conn = &mut redis_pool.get().await?;
        let nb_map = redis_conn.get(mappack_nb_map_key(&self.mappack_id)).await?;
        Ok(nb_map)
    }

    async fn leaderboard<'a>(
        &'a self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Vec<MappackPlayer<'a>>> {
        let redis_pool = ctx.data_unchecked::<RedisPool>();
        let mysql_pool = ctx.data_unchecked::<MySqlPool>();

        let redis_conn = &mut redis_pool.get().await?;
        let mysql_conn = &mut mysql_pool.acquire().await?;

        let leaderboard: Vec<u32> = redis_conn
            .zrange(mappack_lb_key(&self.mappack_id), 0, -1)
            .await?;

        let mut out = Vec::with_capacity(leaderboard.len());

        for id in leaderboard {
            let player = player::get_player_from_id(&mut **mysql_conn, id).await?;
            out.push(MappackPlayer {
                inner: player.into(),
                mappack_id: &self.mappack_id,
            });
        }

        Ok(out)
    }

    async fn player<'a>(
        &'a self,
        ctx: &async_graphql::Context<'_>,
        login: String,
    ) -> async_graphql::Result<MappackPlayer<'a>> {
        let mysql_pool = ctx.data_unchecked::<MySqlPool>();
        let player = must::have_player(mysql_pool, &login).await?;

        Ok(MappackPlayer {
            inner: player.into(),
            mappack_id: &self.mappack_id,
        })
    }
}

pub async fn calc_scores(
    ctx: &async_graphql::Context<'_>,
    mappack_id: String,
) -> RecordsResult<Mappack> {
    let db = ctx.data_unchecked::<Database>();
    let mysql_conn = &mut db.mysql_pool.acquire().await.with_api_err()?;
    let redis_conn = &mut db.redis_pool.get().await?;
    let mappack_key = mappack_key(&mappack_id);

    let mappack_uids: Vec<String> = redis_conn.smembers(&mappack_key).await.with_api_err()?;

    // We load the campaign, and update it, before retrieving the scores from it
    if mappack_uids.is_empty() {
        let Ok(mappack_id_int) = mappack_id.parse() else {
            return Err(RecordsErrorKind::InvalidMappackId(mappack_id));
        };

        let client = ctx.data_unchecked::<Client>();

        // We fill the mappack
        fill_mappack(client, mysql_conn, redis_conn, &mappack_key, mappack_id_int).await?;

        // And we update it to have its scores cached
        update_mappack(&mappack_id, mysql_conn, redis_conn)
            .await
            .with_api_err()?;
    }

    Ok(Mappack { mappack_id })
}

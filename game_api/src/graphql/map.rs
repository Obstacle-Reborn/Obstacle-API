use std::{collections::HashMap, iter::repeat, sync::Arc};

use async_graphql::{
    dataloader::{DataLoader, Loader},
    ID,
};
use deadpool_redis::redis::AsyncCommands;
use futures::StreamExt;
use records_lib::{
    escaped::Escaped,
    models::{self, Record},
    redis_key::alone_map_key,
    update_ranks::{get_rank_or_full_update, update_leaderboard},
    Database, GetSqlFragments as _,
};
use sqlx::{mysql, FromRow, MySqlPool};

use crate::auth::{self, privilege, WebToken};

use super::{
    medal::MedalPrice,
    player::{Player, PlayerLoader},
    rating::{PlayerRating, Rating},
    record::RankedRecord,
    SortState,
};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Map {
    #[sqlx(flatten)]
    pub inner: models::Map,
}

impl From<models::Map> for Map {
    fn from(inner: models::Map) -> Self {
        Self { inner }
    }
}

impl Map {
    pub(super) async fn get_records(
        &self,
        ctx: &async_graphql::Context<'_>,
        rank_sort_by: Option<SortState>,
        date_sort_by: Option<SortState>,
        event: Option<(&models::Event, &models::EventEdition)>,
    ) -> async_graphql::Result<Vec<RankedRecord>> {
        let db = ctx.data_unchecked::<Database>();
        let mysql_conn = &mut db.mysql_pool.acquire().await?;
        let redis_conn = &mut db.redis_pool.get().await?;

        let key = alone_map_key(self.inner.id);
        let reversed = self.inner.reversed.unwrap_or(false);

        update_leaderboard((mysql_conn, redis_conn), &self.inner, None).await?;

        let to_reverse = reversed
            ^ rank_sort_by
                .as_ref()
                .filter(|s| **s == SortState::Reverse)
                .is_some();
        let record_ids: Vec<i32> = if to_reverse {
            redis_conn.zrevrange(&key, 0, 99)
        } else {
            redis_conn.zrange(&key, 0, 99)
        }
        .await?;
        if record_ids.is_empty() {
            return Ok(Vec::new());
        }

        let player_ids_query = record_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");

        let (and_player_id_in, order_by_clause) = if let Some(s) = date_sort_by.as_ref() {
            let date_sort_by = if *s == SortState::Reverse {
                "ASC".to_owned()
            } else {
                "DESC".to_owned()
            };
            (String::new(), format!("r.record_date {date_sort_by}"))
        } else {
            (
                format!("AND r.record_player_id IN ({player_ids_query})"),
                format!(
                    "r.time {order}, r.record_date ASC",
                    order = if to_reverse { "DESC" } else { "ASC" }
                ),
            )
        };

        let (join_event, and_event) = event.get_sql_fragments();

        // Query the records with these ids
        let query = format!(
            "SELECT * FROM global_records r
            {join_event}
            WHERE map_id = ? {and_player_id_in}
            {and_event}
            ORDER BY {order_by_clause}
            {limit}",
            limit = if date_sort_by.is_some() || rank_sort_by.is_some() {
                "LIMIT 100"
            } else {
                ""
            }
        );

        let mut query = sqlx::query_as::<_, Record>(&query).bind(self.inner.id);
        if date_sort_by.is_none() {
            for id in &record_ids {
                query = query.bind(id);
            }
        }

        if let Some((event, edition)) = event {
            query = query.bind(event.id).bind(edition.id);
        }

        let mut records = query.fetch(&mut **mysql_conn);
        let mut ranked_records = Vec::with_capacity(records.size_hint().0);

        let mysql_conn = &mut db.mysql_pool.acquire().await?;

        while let Some(record) = records.next().await {
            let record = record?;
            let rank = get_rank_or_full_update(
                (&mut *mysql_conn, redis_conn),
                &self.inner,
                record.time,
                None,
            )
            .await?;

            ranked_records.push(models::RankedRecord { rank, record }.into());
        }

        Ok(ranked_records)
    }
}

#[async_graphql::Object]
impl Map {
    pub async fn id(&self) -> ID {
        ID(format!("v0:Map:{}", self.inner.id))
    }

    async fn game_id(&self) -> Escaped {
        self.inner.game_id.clone().into()
    }

    async fn player_id(&self) -> ID {
        ID(format!("v0:Player:{}", self.inner.player_id))
    }

    async fn cps_number(&self) -> Option<u32> {
        self.inner.cps_number
    }

    async fn player(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Player> {
        ctx.data_unchecked::<DataLoader<PlayerLoader>>()
            .load_one(self.inner.player_id)
            .await?
            .ok_or_else(|| async_graphql::Error::new("Player not found."))
    }

    async fn name(&self) -> Escaped {
        self.inner.name.clone().into()
    }

    async fn reversed(&self) -> bool {
        self.inner.reversed.unwrap_or(false)
    }

    async fn medal_for(
        &self,
        ctx: &async_graphql::Context<'_>,
        req_login: String,
    ) -> async_graphql::Result<Option<MedalPrice>> {
        let Some(WebToken { login, token }) = ctx.data_opt::<WebToken>() else {
            return Err(async_graphql::Error::new("Unauthorized."));
        };

        let role = if *login != req_login {
            privilege::ADMIN
        } else {
            privilege::PLAYER
        };

        let db = ctx.data_unchecked();

        auth::website_check_auth_for(db, login, token, role).await?;

        let player_id: u32 = sqlx::query_scalar("SELECT id FROM players WHERE login = ?")
            .bind(login)
            .fetch_one(&db.mysql_pool)
            .await?;

        Ok(sqlx::query_as(
            "SELECT * FROM prizes WHERE map_id = ? AND player_id = ? ORDER BY medal DESC",
        )
        .bind(self.inner.id)
        .bind(player_id)
        .fetch_optional(&db.mysql_pool)
        .await?)
    }

    async fn ratings(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Vec<Rating>> {
        let db: &Database = ctx.data_unchecked();
        let Some(WebToken { login, token }) = ctx.data_opt::<WebToken>() else {
            return Err(async_graphql::Error::new("Unauthorized."));
        };

        let author_login: String = sqlx::query_scalar("SELECT login FROM player WHERE id = ?")
            .bind(self.inner.player_id)
            .fetch_one(&db.mysql_pool)
            .await?;

        let role = if author_login != *login {
            privilege::ADMIN
        } else {
            privilege::PLAYER
        };

        auth::website_check_auth_for(db, login, token, role).await?;

        Ok(sqlx::query_as("SELECT * FROM rating WHERE map_id = ?")
            .bind(self.inner.id)
            .fetch_all(&db.mysql_pool)
            .await?)
    }

    async fn average_rating(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Vec<PlayerRating>> {
        let db = ctx.data_unchecked();
        let fetch_all = sqlx::query_as(
            r#"SELECT CAST(0 AS UNSIGNED) AS "player_id", map_id, kind,
            AVG(rating) AS "rating" FROM player_rating
            WHERE map_id = ? GROUP BY kind ORDER BY kind"#,
        )
        .bind(self.inner.id)
        .fetch_all(db)
        .await?;
        Ok(fetch_all)
    }

    #[inline(always)]
    async fn records(
        &self,
        ctx: &async_graphql::Context<'_>,
        rank_sort_by: Option<SortState>,
        date_sort_by: Option<SortState>,
    ) -> async_graphql::Result<Vec<RankedRecord>> {
        self.get_records(ctx, rank_sort_by, date_sort_by, None)
            .await
    }
}

pub struct MapLoader(pub MySqlPool);

#[async_graphql::async_trait::async_trait]
impl Loader<u32> for MapLoader {
    type Value = Map;
    type Error = Arc<sqlx::Error>;

    async fn load(&self, keys: &[u32]) -> Result<HashMap<u32, Self::Value>, Self::Error> {
        let query = format!(
            "SELECT * FROM maps WHERE id IN ({})",
            repeat("?".to_string())
                .take(keys.len())
                .collect::<Vec<_>>()
                .join(",")
        );

        let mut query = sqlx::query(&query);

        for key in keys {
            query = query.bind(key);
        }

        Ok(query
            .map(|row: mysql::MySqlRow| {
                let map = Map::from_row(&row).unwrap();
                (map.inner.id, map)
            })
            .fetch_all(&self.0)
            .await?
            .into_iter()
            .collect())
    }
}

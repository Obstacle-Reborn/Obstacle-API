use std::{collections::HashMap, sync::Arc};

use async_graphql::{connection, dataloader::Loader, Context, Enum, ID};
use futures::StreamExt;
use records_lib::{
    escaped::Escaped, models::{self, RecordAttr, Role}, Database
};
use sqlx::{mysql, FromRow, MySqlPool, Row};

use crate::RecordsErrorKind;

use super::{
    ban::Banishment,
    get_rank_or_full_update,
    map::Map,
    record::RankedRecord,
    utils::{
        connections_append_query_string_order, connections_append_query_string_page,
        connections_bind_query_parameters_order, connections_bind_query_parameters_page,
        connections_pages_info, decode_id,
    },
    SortState,
};

#[derive(Copy, Clone, Eq, PartialEq, Enum)]
#[repr(u8)]
enum PlayerRole {
    Player = 0,
    Moderator = 1,
    Admin = 2,
}

impl TryFrom<Role> for PlayerRole {
    type Error = RecordsErrorKind;

    fn try_from(role: Role) -> Result<Self, Self::Error> {
        if role.id < 3 {
            // SAFETY: enum is repr(u8) and role id is in range
            Ok(unsafe { std::mem::transmute(role.id) })
        } else {
            Err(RecordsErrorKind::UnknownRole(role.id, role.role_name))
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Player {
    #[sqlx(flatten)]
    pub inner: models::Player,
}

impl From<models::Player> for Player {
    fn from(inner: models::Player) -> Self {
        Self { inner }
    }
}

#[async_graphql::Object]
impl Player {
    pub async fn id(&self) -> ID {
        ID(format!("v0:Player:{}", self.inner.id))
    }

    async fn login(&self) -> Escaped {
        self.inner.login.clone().into()
    }

    async fn name(&self) -> Escaped {
        self.inner.name.clone().into()
    }

    async fn zone_path(&self) -> Option<&str> {
        self.inner.zone_path.as_deref()
    }

    async fn banishments(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<Banishment>> {
        let db = ctx.data_unchecked::<MySqlPool>();
        Ok(
            sqlx::query_as("SELECT * FROM banishments WHERE player_id = ?")
                .bind(self.inner.id)
                .fetch_all(db)
                .await?,
        )
    }

    async fn role(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<PlayerRole> {
        let db = ctx.data_unchecked::<MySqlPool>();

        let r: Role = sqlx::query_as("SELECT * FROM role WHERE id = ?")
            .bind(self.inner.role)
            .fetch_one(db)
            .await?;

        Ok(r.try_into()?)
    }

    async fn maps(
        &self,
        ctx: &async_graphql::Context<'_>,
        after: Option<String>,
        before: Option<String>,
        first: Option<i32>,
        last: Option<i32>,
    ) -> async_graphql::Result<connection::Connection<ID, Map>> {
        connection::query(
            after,
            before,
            first,
            last,
            |after: Option<ID>, before: Option<ID>, first: Option<usize>, last: Option<usize>| async move {
                let after = decode_id(after.as_ref());
                let before = decode_id(before.as_ref());

                // Build the query string
                let mut query = String::from("SELECT m1.* FROM maps m1 WHERE player_id = ? ");
                connections_append_query_string_page(&mut query, true, after, before);
                query.push_str(" GROUP BY name HAVING id = (SELECT MAX(id) FROM maps m2 WHERE player_id = ? AND m1.name = m2.name)");
                connections_append_query_string_order(&mut query, first, last);

                let reversed = first.is_none() && last.is_some();

                // Bind the parameters
                let mut query = sqlx::query(&query);
                query = query.bind(self.inner.id);
                query = connections_bind_query_parameters_page(query, after, before);
                query = query.bind(self.inner.id);
                query = connections_bind_query_parameters_order(query, first, last);

                // Execute the query
                let mysql_pool = ctx.data_unchecked::<MySqlPool>();
                let mut maps =
                    query
                        .map(|x: mysql::MySqlRow| {
                            let cursor = ID(format!("v0:Map:{}", x.get::<u32, _>(0)));
                            connection::Edge::new(cursor, models::Map::from_row(&x).unwrap().into())
                        })
                        .fetch_all(mysql_pool)
                        .await?;

                if reversed {
                    maps.reverse();
                }

                let (has_previous_page, has_next_page) = connections_pages_info(maps.len(), first, last);
                let mut connection = connection::Connection::new(has_previous_page, has_next_page);
                connection.edges.extend(maps);
                Ok::<_, sqlx::Error>(connection)
            },
        )
        .await
    }

    async fn records(
        &self,
        ctx: &async_graphql::Context<'_>,
        date_sort_by: Option<SortState>,
    ) -> async_graphql::Result<Vec<RankedRecord>> {
        let db = ctx.data_unchecked::<Database>();
        let mysql_conn = &mut db.mysql_pool.acquire().await?;
        let redis_conn = &mut db.redis_pool.get().await?;

        let date_sort_by = SortState::sql_order_by(&date_sort_by);

        // Query the records with these ids

        let query = format!(
            "SELECT * FROM global_records
            WHERE record_player_id = ? AND game_id NOT LIKE '%_benchmark'
            ORDER BY record_date {date_sort_by}
            LIMIT 100",
        );

        let mut records = sqlx::query_as::<_, RecordAttr>(&query)
            .bind(self.inner.id)
            .fetch(&mut **mysql_conn);

        let mut ranked_records = Vec::with_capacity(records.size_hint().0);

        let mysql_conn = &mut db.mysql_pool.acquire().await?;

        while let Some(record) = records.next().await {
            let RecordAttr { record, map } = record?;

            let rank =
                get_rank_or_full_update((mysql_conn, redis_conn), &map, record.time, None).await?;

            ranked_records.push(models::RankedRecord { rank, record }.into());
        }

        Ok(ranked_records)
    }
}

pub struct PlayerLoader(pub MySqlPool);

#[async_graphql::async_trait::async_trait]
impl Loader<u32> for PlayerLoader {
    type Value = Player;
    type Error = Arc<sqlx::Error>;

    async fn load(&self, keys: &[u32]) -> Result<HashMap<u32, Self::Value>, Self::Error> {
        let query = format!(
            "SELECT * FROM players WHERE id IN ({})",
            keys.iter()
                .map(|_| "?".to_string())
                .collect::<Vec<String>>()
                .join(",")
        );

        let mut query = sqlx::query(&query);

        for key in keys {
            query = query.bind(key);
        }

        Ok(query
            .map(|row: mysql::MySqlRow| {
                let player = Player::from_row(&row).unwrap();
                (player.inner.id, player)
            })
            .fetch_all(&self.0)
            .await?
            .into_iter()
            .collect())
    }
}

use async_graphql::{dataloader::DataLoader, Context};
use sqlx::MySqlPool;

use records_lib::models::{self, Medal};

use super::{
    map::{Map, MapLoader},
    player::{Player, PlayerLoader},
};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MedalPrice {
    #[sqlx(flatten)]
    inner: models::MedalPrice,
}

impl From<models::MedalPrice> for MedalPrice {
    fn from(inner: models::MedalPrice) -> Self {
        Self { inner }
    }
}

#[async_graphql::Object]
impl MedalPrice {
    async fn price_date(&self) -> chrono::NaiveDateTime {
        self.inner.price_date
    }

    async fn medal(&self, ctx: &Context<'_>) -> async_graphql::Result<Medal> {
        let db = ctx.data_unchecked::<MySqlPool>();
        Ok(sqlx::query_as("SELECT * FROM medal_type WHERE id = ?")
            .bind(self.inner.medal)
            .fetch_one(db)
            .await?)
    }

    async fn map(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Map> {
        ctx.data_unchecked::<DataLoader<MapLoader>>()
            .load_one(self.inner.map_id)
            .await?
            .ok_or_else(|| async_graphql::Error::new("Voter player not found."))
    }

    async fn player(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Player> {
        ctx.data_unchecked::<DataLoader<PlayerLoader>>()
            .load_one(self.inner.player_id)
            .await?
            .ok_or_else(|| async_graphql::Error::new("Voter player not found."))
    }
}

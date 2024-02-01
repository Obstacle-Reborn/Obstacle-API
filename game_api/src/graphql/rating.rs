use async_graphql::{dataloader::DataLoader, Context};
use records_lib::models::{self, RatingKind};

use super::{
    map::{Map, MapLoader},
    player::{Player, PlayerLoader},
};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Rating {
    #[sqlx(flatten)]
    inner: models::Rating,
}

impl From<models::Rating> for Rating {
    fn from(inner: models::Rating) -> Self {
        Self { inner }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PlayerRating {
    #[sqlx(flatten)]
    inner: models::PlayerRating,
}

impl From<models::PlayerRating> for PlayerRating {
    fn from(inner: models::PlayerRating) -> Self {
        Self { inner }
    }
}

#[async_graphql::Object]
impl Rating {
    async fn ratings(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<PlayerRating>> {
        let db = ctx.data_unchecked();
        Ok(
            sqlx::query_as("SELECT * FROM player_rating WHERE player_id = ? AND map_id = ?")
                .bind(self.inner.player_id)
                .bind(self.inner.map_id)
                .fetch_all(db)
                .await?,
        )
    }

    async fn player(&self, ctx: &Context<'_>) -> async_graphql::Result<Player> {
        ctx.data_unchecked::<DataLoader<PlayerLoader>>()
            .load_one(self.inner.player_id)
            .await?
            .ok_or_else(|| async_graphql::Error::new("Voter player not found."))
    }

    async fn map(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Map> {
        ctx.data_unchecked::<DataLoader<MapLoader>>()
            .load_one(self.inner.map_id)
            .await?
            .ok_or_else(|| async_graphql::Error::new("Voter player not found."))
    }

    async fn rating_date(&self) -> chrono::NaiveDateTime {
        self.inner.rating_date
    }
}

#[async_graphql::Object]
impl PlayerRating {
    async fn kind(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<RatingKind> {
        let db = ctx.data_unchecked();
        Ok(sqlx::query_as("SELECT * FROM rating_kind WHERE id = ?")
            .bind(self.inner.kind)
            .fetch_one(db)
            .await?)
    }

    async fn rating(&self) -> f32 {
        self.inner.rating
    }
}

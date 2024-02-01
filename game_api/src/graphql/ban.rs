use async_graphql::{dataloader::DataLoader, Context};

use records_lib::models;

use super::player::{Player, PlayerLoader};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Banishment {
    #[sqlx(flatten)]
    inner: models::Banishment,
}

impl From<models::Banishment> for Banishment {
    fn from(inner: models::Banishment) -> Self {
        Self { inner }
    }
}

#[async_graphql::Object]
impl Banishment {
    async fn id(&self) -> u32 {
        self.inner.inner.id
    }

    async fn date_ban(&self) -> chrono::NaiveDateTime {
        self.inner.inner.date_ban
    }

    async fn duration(&self) -> i64 {
        self.inner.inner.duration
    }

    async fn was_reprieved(&self) -> bool {
        self.inner.was_reprieved
    }

    async fn reason(&self) -> &str {
        &self.inner.inner.reason
    }

    async fn player(&self, ctx: &Context<'_>) -> async_graphql::Result<Player> {
        ctx.data_unchecked::<DataLoader<PlayerLoader>>()
            .load_one(self.inner.inner.player_id)
            .await?
            .ok_or_else(|| async_graphql::Error::new("Banned player not found."))
    }

    async fn banished_by(&self, ctx: &Context<'_>) -> async_graphql::Result<Player> {
        ctx.data_unchecked::<DataLoader<PlayerLoader>>()
            .load_one(self.inner.inner.banished_by)
            .await?
            .ok_or_else(|| async_graphql::Error::new("Banner player not found."))
    }
}

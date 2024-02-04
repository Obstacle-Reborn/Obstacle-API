use std::{collections::HashMap, iter::repeat, sync::Arc};

use async_graphql::{
    dataloader::{DataLoader, Loader},
    Context,
};
use sqlx::{mysql, FromRow, MySqlPool};

use records_lib::{
    models::{self, EventCategory},
    must,
};

use super::{mappack::Mappack, player::Player};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Event {
    #[sqlx(flatten)]
    inner: models::Event,
}

impl From<models::Event> for Event {
    fn from(inner: models::Event) -> Self {
        Self { inner }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventEdition {
    #[sqlx(flatten)]
    inner: models::EventEdition,
}

impl From<models::EventEdition> for EventEdition {
    fn from(inner: models::EventEdition) -> Self {
        Self { inner }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventEditionMaps {
    #[sqlx(flatten)]
    inner: models::EventEditionMaps,
}

impl From<models::EventEditionMaps> for EventEditionMaps {
    fn from(inner: models::EventEditionMaps) -> Self {
        Self { inner }
    }
}

#[async_graphql::Object]
impl Event {
    async fn handle(&self) -> &str {
        &self.inner.handle
    }

    async fn cooldown(&self) -> Option<u8> {
        self.inner.cooldown
    }

    async fn admins(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<Player>> {
        let db = ctx.data_unchecked::<MySqlPool>();
        let q = sqlx::query_as(
            "SELECT * FROM players
            WHERE id IN (
                SELECT player_id FROM event_admins
                WHERE event_id = ?
            )",
        )
        .bind(self.inner.id)
        .fetch_all(db)
        .await?;

        Ok(q)
    }

    async fn categories(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<EventCategory>> {
        let db = ctx.data_unchecked::<MySqlPool>();
        let q = sqlx::query_as(
            "SELECT * FROM event_category
            WHERE id IN (
                SELECT category_id FROM event_categories
                WHERE event_id = ?
            )",
        )
        .bind(self.inner.id)
        .fetch_all(db)
        .await?;
        Ok(q)
    }

    async fn editions(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<EventEdition>> {
        let db = ctx.data_unchecked::<MySqlPool>();
        let q = sqlx::query_as("SELECT * FROM event_edition WHERE event_id = ?")
            .bind(self.inner.id)
            .fetch_all(db)
            .await?;
        Ok(q)
    }

    async fn edition(
        &self,
        ctx: &Context<'_>,
        edition_id: u32,
    ) -> async_graphql::Result<EventEdition> {
        let db = ctx.data_unchecked::<MySqlPool>();
        let mysql_conn = &mut db.acquire().await?;
        let (_, edition) =
            must::have_event_edition(mysql_conn, &self.inner.handle, edition_id).await?;
        Ok(edition.into())
    }
}

pub struct EventLoader(pub MySqlPool);

#[async_graphql::async_trait::async_trait]
impl Loader<u32> for EventLoader {
    type Value = Event;
    type Error = Arc<sqlx::Error>;

    async fn load(&self, keys: &[u32]) -> Result<HashMap<u32, Self::Value>, Self::Error> {
        let q = format!(
            "SELECT * FROM event WHERE id IN ({})",
            repeat("?".to_string())
                .take(keys.len())
                .collect::<Vec<_>>()
                .join(",")
        );

        let mut q = sqlx::query(&q);

        for key in keys {
            q = q.bind(key);
        }

        Ok(q.map(|row: mysql::MySqlRow| {
            let event = models::Event::from_row(&row).unwrap();
            (event.id, event.into())
        })
        .fetch_all(&self.0)
        .await?
        .into_iter()
        .collect())
    }
}

pub struct EventCategoryLoader(pub MySqlPool);

#[async_graphql::async_trait::async_trait]
impl Loader<u32> for EventCategoryLoader {
    type Value = EventCategory;
    type Error = Arc<sqlx::Error>;

    async fn load(&self, keys: &[u32]) -> Result<HashMap<u32, Self::Value>, Self::Error> {
        let q = format!(
            "SELECT * FROM event_category WHERE id IN ({})",
            repeat("?".to_string())
                .take(keys.len())
                .collect::<Vec<_>>()
                .join(",")
        );

        let mut q = sqlx::query(&q);

        for key in keys {
            q = q.bind(key);
        }

        Ok(q.map(|row: mysql::MySqlRow| {
            let category = EventCategory::from_row(&row).unwrap();
            (category.id, category)
        })
        .fetch_all(&self.0)
        .await?
        .into_iter()
        .collect())
    }
}

#[async_graphql::Object]
impl EventEdition {
    async fn id(&self) -> u32 {
        self.inner.id
    }

    async fn mappack(&self) -> Option<Mappack> {
        let mappack_id = self.inner.mx_id?.to_string();
        Some(Mappack { mappack_id })
    }

    async fn event(&self, ctx: &Context<'_>) -> async_graphql::Result<Event> {
        ctx.data_unchecked::<DataLoader<EventLoader>>()
            .load_one(self.inner.event_id)
            .await?
            .ok_or_else(|| async_graphql::Error::new("Event not found."))
    }

    async fn name(&self) -> &str {
        &self.inner.name
    }

    async fn start_date(&self) -> &chrono::NaiveDateTime {
        &self.inner.start_date
    }

    async fn banner_img_url(&self) -> Option<&str> {
        self.inner.banner_img_url.as_deref()
    }

    async fn categories(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<EventCategory>> {
        let db = ctx.data_unchecked::<MySqlPool>();
        let q = sqlx::query_as(
            "SELECT * FROM event_category
            WHERE id IN (
                SELECT category_id FROM event_edition_categories
                WHERE event_id = ? AND edition_id = ?
            )",
        )
        .bind(self.inner.event_id)
        .bind(self.inner.id)
        .fetch_all(db)
        .await?;
        Ok(q)
    }
}

#[async_graphql::Object]
impl EventEditionMaps {
    async fn event(&self, ctx: &Context<'_>) -> async_graphql::Result<Event> {
        ctx.data_unchecked::<DataLoader<EventLoader>>()
            .load_one(self.inner.event_id)
            .await?
            .ok_or_else(|| async_graphql::Error::new("Event not found."))
    }

    async fn edition(&self, ctx: &Context<'_>) -> async_graphql::Result<EventEdition> {
        let db = ctx.data_unchecked::<MySqlPool>();
        let q = sqlx::query_as("SELECT * FROM event_edition WHERE id = ? AND event_id = ?")
            .bind(self.inner.edition_id)
            .bind(self.inner.event_id)
            .fetch_one(db)
            .await?;
        Ok(q)
    }

    async fn category(&self, ctx: &Context<'_>) -> async_graphql::Result<Option<EventCategory>> {
        let Some(category_id) = self.inner.category_id else {
            return Ok(None);
        };

        Ok(Some(
            ctx.data_unchecked::<DataLoader<EventCategoryLoader>>()
                .load_one(category_id)
                .await?
                .ok_or_else(|| async_graphql::Error::new("Category not found."))?,
        ))
    }
}

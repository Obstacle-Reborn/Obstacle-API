use std::fmt;

use async_graphql::{Enum, SimpleObject};
use serde::Serialize;
use sqlx::{mysql::MySqlRow, FromRow, Row};

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct Player {
    pub id: u32,
    pub login: String,
    pub name: String,
    pub join_date: Option<chrono::NaiveDateTime>,
    pub zone_path: Option<String>,
    pub admins_note: Option<String>,
    pub role: u8,
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct PlayerIps {
    pub player_id: u32,
    pub ip_hash: Vec<u8>,
    pub ip_date: chrono::NaiveDateTime,
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct Map {
    pub id: u32,
    pub game_id: String,
    pub player_id: u32,
    pub name: String,
    pub cps_number: Option<u32>,
    pub linked_map: Option<u32>,
    pub reversed: Option<bool>,
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct Record {
    pub record_id: u32,
    pub record_player_id: u32,
    pub map_id: u32,
    pub time: i32,
    pub respawn_count: i32,
    pub record_date: chrono::NaiveDateTime,
    pub flags: u32,
    pub try_count: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct RankedRecord {
    pub rank: i32,
    pub record: Record,
}

/// This is the type returned from the `global_records` SQL view.
#[derive(FromRow)]
pub struct RecordAttr {
    #[sqlx(flatten)]
    pub record: Record,
    #[sqlx(flatten)]
    pub map: Map,
}

#[derive(Serialize, FromRow, Clone, PartialEq, Eq, Debug, PartialOrd, Ord)]
#[non_exhaustive]
pub struct Role {
    pub id: u8,
    pub role_name: String,
    pub privileges: u8,
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct BanishmentInner {
    pub id: u32,
    pub date_ban: chrono::NaiveDateTime,
    pub duration: i64,
    pub reason: String,
    pub player_id: u32,
    pub banished_by: u32,
}

#[derive(Serialize, Clone, Debug)]
pub struct Banishment {
    #[serde(flatten)]
    pub inner: BanishmentInner,
    pub was_reprieved: bool,
}

impl<'r> FromRow<'r, MySqlRow> for Banishment {
    fn from_row(row: &'r MySqlRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            inner: BanishmentInner::from_row(row)?,
            was_reprieved: row.try_get::<i8, _>("was_reprieved")? != 0,
        })
    }
}

impl fmt::Display for BanishmentInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "at: {:?}, duration: {}, reason: `{}`",
            self.date_ban,
            if self.duration == -1 {
                "forever".to_owned()
            } else {
                format!("{} seconds", self.duration)
            },
            if self.reason.is_empty() {
                "none"
            } else {
                &self.reason
            }
        )
    }
}

impl fmt::Display for Banishment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <BanishmentInner as fmt::Display>::fmt(&self.inner, f)
    }
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct Checkpoint {
    pub cp_num: u32,
    pub map_id: u32,
}

#[derive(Serialize, FromRow, Clone, Debug, SimpleObject)]
pub struct CheckpointTimes {
    pub cp_num: u32,
    #[graphql(skip)]
    pub map_id: u32,
    #[graphql(skip)]
    pub record_id: u32,
    pub time: i32,
}

#[derive(Serialize, PartialEq, Eq, Clone, Copy, Debug, Enum)]
#[non_exhaustive]
pub enum Medal {
    Bronze,
    Silver,
    Gold,
    Champion,
}

impl<'r> FromRow<'r, MySqlRow> for Medal {
    fn from_row(row: &'r MySqlRow) -> Result<Self, sqlx::Error> {
        let id: u8 = row.try_get("id")?;
        let medal_name: String = row.try_get("medal_name")?;

        match (id, &*medal_name) {
            (0, "bronze") => Ok(Self::Bronze),
            (1, "silver") => Ok(Self::Silver),
            (2, "gold") => Ok(Self::Gold),
            (3, "champion") => Ok(Self::Champion),
            (id, _) => Err(sqlx::Error::ColumnDecode {
                index: "id".to_owned(),
                source: format!("unknown medal `{id}`, `{medal_name}`").into(),
            }),
        }
    }
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct MedalPrice {
    pub medal: u8,
    pub map_id: u32,
    pub player_id: u32,
    pub price_date: chrono::NaiveDateTime,
}

#[derive(Serialize, PartialEq, Eq, Clone, Copy, Debug, Enum)]
#[non_exhaustive]
pub enum RatingKind {
    Route,
    Deco,
    Smoothness,
    Difficulty,
}

impl<'r> FromRow<'r, MySqlRow> for RatingKind {
    fn from_row(row: &'r MySqlRow) -> Result<Self, sqlx::Error> {
        let id: u8 = row.try_get("id")?;
        let kind: String = row.try_get("kind")?;

        match (id, &*kind) {
            (0, "route") => Ok(Self::Route),
            (1, "deco") => Ok(Self::Deco),
            (2, "smoothness") => Ok(Self::Smoothness),
            (3, "difficulty") => Ok(Self::Difficulty),
            (id, _) => Err(sqlx::Error::ColumnDecode {
                index: "id".to_owned(),
                source: format!("unknown rating kind: `{id}`, `{kind}`").into(),
            }),
        }
    }
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct Rating {
    pub player_id: u32,
    pub map_id: u32,
    pub rating_date: chrono::NaiveDateTime,
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct PlayerRating {
    pub player_id: u32,
    pub map_id: u32,
    pub kind: u8,
    pub rating: f32,
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct Event {
    pub id: u32,
    pub handle: String,
    pub cooldown: Option<u8>,
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct EventAdmins {
    pub event_id: u32,
    pub player_id: u32,
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct EventCategories {
    pub event_id: u32,
    pub category_id: u32,
}

#[derive(Serialize, FromRow, Clone, Debug, SimpleObject, Default)]
pub struct EventCategory {
    #[graphql(skip)]
    pub id: u32,
    pub handle: String,
    pub name: String,
    pub banner_img_url: Option<String>,
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct EventEdition {
    pub id: u32,
    pub event_id: u32,
    pub name: String,
    pub start_date: chrono::NaiveDateTime,
    pub banner_img_url: Option<String>,
    pub mx_id: Option<i32>,
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct EventEditionCategories {
    pub event_id: u32,
    pub edition_id: u32,
    pub category_id: u32,
}

#[derive(Serialize, FromRow, Clone, Debug)]
pub struct EventEditionMaps {
    pub event_id: u32,
    pub edition_id: u32,
    pub map_id: u32,
    pub category_id: Option<u32>,
    pub mx_id: i64,
}

#[derive(Serialize, PartialEq, Eq, Clone, Copy, Debug)]
pub enum ApiStatusKind {
    Normal,
    Maintenance,
}

impl<'r> FromRow<'r, MySqlRow> for ApiStatusKind {
    fn from_row(row: &'r MySqlRow) -> Result<Self, sqlx::Error> {
        let id: u8 = row.try_get("status_id")?;
        let kind: String = row.try_get("status_name")?;

        match (id, &*kind) {
            (1, "normal") => Ok(Self::Normal),
            (2, "maintenance") => Ok(Self::Maintenance),
            (id, _) => Err(sqlx::Error::ColumnDecode {
                index: "status_id".to_owned(),
                source: format!("unknown status `{id}`, `{kind}`").into(),
            }),
        }
    }
}

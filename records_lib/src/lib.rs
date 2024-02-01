use core::fmt;
use std::{env, fs::read_to_string, str::FromStr, time::Duration};

use deadpool_redis::Runtime;
use sqlx::{MySql, Pool};

pub mod error;
pub mod escaped;
pub mod models;
pub mod must;
pub mod redis_key;
pub mod update_mappacks;
pub mod update_ranks;

pub mod event;
pub mod map;
pub mod player;

pub type MySqlPool = Pool<MySql>;
pub type RedisPool = deadpool_redis::Pool;
pub type RedisConnection = deadpool_redis::Connection;

pub trait GetSqlFragments {
    fn get_sql_fragments(self) -> (&'static str, &'static str);
}

fn get_sql_fragments() -> (&'static str, &'static str) {
    (
        "INNER JOIN event_edition_records eer ON r.record_id = eer.record_id",
        "AND eer.event_id = ? AND eer.edition_id = ?",
    )
}

impl GetSqlFragments for Option<&(models::Event, models::EventEdition)> {
    fn get_sql_fragments(self) -> (&'static str, &'static str) {
        self.is_some().then(get_sql_fragments).unwrap_or_default()
    }
}

#[derive(Clone)]
pub struct Database {
    pub mysql_pool: MySqlPool,
    pub redis_pool: RedisPool,
}

pub async fn get_mysql_pool() -> anyhow::Result<MySqlPool> {
    let mysql_pool = sqlx::mysql::MySqlPoolOptions::new().acquire_timeout(Duration::from_secs(10));

    #[cfg(feature = "localhost_test")]
    let url = &get_env_var("DATABASE_URL");
    #[cfg(not(feature = "localhost_test"))]
    let url = &read_env_var_file("DATABASE_URL");

    let mysql_pool = mysql_pool.connect(url).await?;
    Ok(mysql_pool)
}

pub fn get_redis_pool() -> anyhow::Result<RedisPool> {
    let cfg = deadpool_redis::Config {
        url: Some(get_env_var("REDIS_URL")),
        connection: None,
        pool: None,
    };
    Ok(cfg.create_pool(Some(Runtime::Tokio1))?)
}

/// Retrieves the string value of the given environment variable.
///
/// # Panic
///
/// This function panics at runtime if it fails to read the environment variable. It has this
/// behavior because all the environment variable of the Records API are read as configuration.
pub fn get_env_var(v: &str) -> String {
    env::var(v).unwrap_or_else(|e| panic!("unable to retrieve env var {v}: {e:?}"))
}

pub fn get_env_var_as<T>(v: &str) -> T
where
    T: FromStr,
    <T as FromStr>::Err: fmt::Debug,
{
    get_env_var(v).parse().unwrap_or_else(|e| {
        panic!(
            "unable to parse {v} env var to {}: {e:?}",
            std::any::type_name::<T>()
        )
    })
}

/// Reads the content of the path specified in the given environment variable, and returns it.
///
/// # Panic
///
/// This function panics at runtime if it fails to read the environment variable. It has this
/// behavior because all the environment variable of the Records API are read as configuration.
pub fn read_env_var_file(v: &str) -> String {
    read_to_string(get_env_var(v)).unwrap_or_else(|e| panic!("unable to read from {v} path: {e:?}"))
}

use core::fmt;

use deadpool_redis::redis::{RedisWrite, ToRedisArgs};

use crate::models;

const V3_KEY_PREFIX: &str = "v3";

const V3_MAPPACK_KEY_PREFIX: &str = "mappack";
const V3_NO_TTL_MAPPACKS: &str = "no_ttl_mappacks";

const V3_MAP_KEY_PREFIX: &str = "lb";

const V3_EVENT_KEY_PREFIX: &str = "event";

const V3_TOKEN_KEY_PREFIX: &str = "token";
const V3_TOKEN_WEB_KEY_PREFIX: &str = "web";
const V3_TOKEN_MP_KEY_PREFIX: &str = "mp";

const V3_MAPPACK_NB_MAP: &str = "nb_map";
const V3_MAPPACK_MX_USERNAME: &str = "mx_username";
const V3_MAPPACK_MX_NAME: &str = "mx_name";
const V3_MAPPACK_MX_CREATED: &str = "mx_created";
const V3_MAPPACK_LB: &str = "lb";
const V3_MAPPACK_LB_RANK_AVG: &str = "rank_avg";
const V3_MAPPACK_LB_MAP_FINISHED: &str = "map_finished";
const V3_MAPPACK_LB_WORST_RANK: &str = "worst_rank";
const V3_MAPPACK_LB_RANKS: &str = "ranks";

const V3_MAPPACK_MAP_KEY_PREFIX: &str = "map";
const V3_MAPPACK_MAP_LAST_RANK: &str = "last_rank";

macro_rules! create_key {
    (
        struct $name:ident$(<$l:lifetime => $_:lifetime>)? = $fn_name:ident $({
            $($field:ident: $field_ty:ty),* $(,)?
        })?$(;$semicolon:tt)?
        |$self:ident, $f:ident| $fmt_expr:expr
    ) => {
        #[derive(Debug)]
        pub struct $name$(<$l>)? $({
            $(pub $field: $field_ty),*
        })?$($semicolon)?

        #[inline(always)]
        pub fn $fn_name$(<$l>)?($($($field: $field_ty),*)?) -> $name$(<$l>)? {
            $name { $($($field),*)? }
        }

        impl ToRedisArgs for $name$(<$_>)? {
            #[inline(always)]
            fn write_redis_args<W>(&self, out: &mut W)
            where
                W: ?Sized + RedisWrite,
            {
                out.write_arg_fmt(self);
            }
        }

        impl fmt::Display for $name$(<$_>)? {
            fn fmt(&$self, $f: &mut fmt::Formatter<'_>) -> fmt::Result {
                $fmt_expr
            }
        }
    }
}

create_key! {
    struct MappacksKey = mappacks_key;;
    |self, f| write!(f, "{V3_KEY_PREFIX}:{V3_MAPPACK_KEY_PREFIX}")
}

create_key! {
    struct MappackKey<'a => '_> = mappack_key {
        mappack_id: &'a str,
    }
    |self, f| write!(
            f,
            "{V3_KEY_PREFIX}:{V3_MAPPACK_KEY_PREFIX}:{}",
            self.mappack_id
        )
}

create_key! {
    struct NoTtlMappacks = no_ttl_mappacks;;
    |self, f| write!(f, "{V3_KEY_PREFIX}:{V3_NO_TTL_MAPPACKS}")
}

create_key! {
    struct AloneMapKey = alone_map_key {
        map_id: u32,
    }
    |self, f| write!(f, "{V3_KEY_PREFIX}:{V3_MAP_KEY_PREFIX}:{}", self.map_id)
}

#[derive(Debug)]
pub struct EventMapKey<'a> {
    pub map_key: AloneMapKey,
    pub event_handle: &'a str,
    pub edition_id: u32,
}

#[inline(always)]
pub fn event_map_key(map_id: u32, event_handle: &str, edition_id: u32) -> EventMapKey<'_> {
    EventMapKey {
        map_key: alone_map_key(map_id),
        event_handle,
        edition_id,
    }
}

impl ToRedisArgs for EventMapKey<'_> {
    #[inline(always)]
    fn write_redis_args<W>(&self, out: &mut W)
    where
        W: ?Sized + RedisWrite,
    {
        out.write_arg_fmt(self)
    }
}

impl fmt::Display for EventMapKey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{V3_KEY_PREFIX}:{V3_EVENT_KEY_PREFIX}:{}:{}:{}",
            self.event_handle, self.edition_id, self.map_key
        )
    }
}

pub enum MapKey<'a> {
    Alone(AloneMapKey),
    Evented(EventMapKey<'a>),
}

impl ToRedisArgs for MapKey<'_> {
    #[inline(always)]
    fn write_redis_args<W>(&self, out: &mut W)
    where
        W: ?Sized + RedisWrite,
    {
        out.write_arg_fmt(self)
    }
}

impl fmt::Display for MapKey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MapKey::Alone(a) => a.fmt(f),
            MapKey::Evented(b) => b.fmt(f),
        }
    }
}

pub fn map_key<'a>(
    map_id: u32,
    event: Option<(&'a models::Event, &'a models::EventEdition)>,
) -> MapKey<'a> {
    match event {
        Some((event, edition)) => MapKey::Evented(event_map_key(map_id, &event.handle, edition.id)),
        None => MapKey::Alone(alone_map_key(map_id)),
    }
}

pub const MP: bool = false;
pub const WEB: bool = true;

pub struct TokenKey<'a, const IS_WEB: bool> {
    pub login: &'a str,
}

#[inline(always)]
pub fn web_token_key(login: &str) -> TokenKey<'_, WEB> {
    TokenKey { login }
}

#[inline(always)]
pub fn mp_token_key(login: &str) -> TokenKey<'_, MP> {
    TokenKey { login }
}

impl<const IS_WEB: bool> ToRedisArgs for TokenKey<'_, IS_WEB>
where
    Self: fmt::Display,
{
    fn write_redis_args<W>(&self, out: &mut W)
    where
        W: ?Sized + RedisWrite,
    {
        out.write_arg_fmt(self);
    }
}

impl fmt::Display for TokenKey<'_, WEB> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{V3_KEY_PREFIX}:{V3_TOKEN_KEY_PREFIX}:{V3_TOKEN_WEB_KEY_PREFIX}:{}",
            self.login
        )
    }
}

impl fmt::Display for TokenKey<'_, MP> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{V3_KEY_PREFIX}:{V3_TOKEN_KEY_PREFIX}:{V3_TOKEN_MP_KEY_PREFIX}:{}",
            self.login
        )
    }
}

create_key! {
    struct MappackNbMapKey<'a => '_> = mappack_nb_map_key {
        mappack_id: &'a str,
    }
    |self, f| write!(
        f,
        "{V3_KEY_PREFIX}:{V3_MAPPACK_KEY_PREFIX}:{}:{V3_MAPPACK_NB_MAP}",
        self.mappack_id
    )
}

create_key! {
    struct MappackMxUsernameKey<'a => '_> = mappack_mx_username_key {
        mappack_id: &'a str,
    }
    |self, f| write!(f, "{V3_KEY_PREFIX}:{V3_MAPPACK_KEY_PREFIX}:{}:{V3_MAPPACK_MX_USERNAME}",
        self.mappack_id
    )
}

create_key! {
    struct MappackMxNameKey<'a => '_> = mappack_mx_name_key {
        mappack_id: &'a str,
    }
    |self, f| write!(f, "{V3_KEY_PREFIX}:{V3_MAPPACK_KEY_PREFIX}:{}:{V3_MAPPACK_MX_NAME}",
        self.mappack_id
    )
}

create_key! {
    struct MappackMxCreatedKey<'a => '_> = mappack_mx_created_key {
        mappack_id: &'a str,
    }
    |self, f| write!(f, "{V3_KEY_PREFIX}:{V3_MAPPACK_KEY_PREFIX}:{}:{V3_MAPPACK_MX_CREATED}",
        self.mappack_id
    )
}

create_key! {
    struct MappackLbKey<'a => '_> = mappack_lb_key {
        mappack_id: &'a str,
    }
    |self, f| write!(
        f,
        "{V3_KEY_PREFIX}:{V3_MAPPACK_KEY_PREFIX}:{}:{V3_MAPPACK_LB}",
        self.mappack_id
    )
}

create_key! {
    struct MappackPlayerRankAvg<'a => '_> = mappack_player_rank_avg_key {
        mappack_id: &'a str,
        player_id: u32,
    }
    |self, f| write!(
        f,
        "{V3_KEY_PREFIX}:{V3_MAPPACK_KEY_PREFIX}:{}:{V3_MAPPACK_LB}:{}:{V3_MAPPACK_LB_RANK_AVG}",
        self.mappack_id, self.player_id
    )
}

create_key! {
    struct MappackPlayerMapFinished<'a => '_> = mappack_player_map_finished_key {
        mappack_id: &'a str,
        player_id: u32,
    }
    |self, f| write!(
        f,
        "{V3_KEY_PREFIX}:{V3_MAPPACK_KEY_PREFIX}:{}:{V3_MAPPACK_LB}:{}:{V3_MAPPACK_LB_MAP_FINISHED}",
        self.mappack_id, self.player_id
    )
}

create_key! {
    struct MappackPlayerWorstRank<'a => '_> = mappack_player_worst_rank_key {
        mappack_id: &'a str,
        player_id: u32,
    }
    |self, f| write!(
        f,
        "{V3_KEY_PREFIX}:{V3_MAPPACK_KEY_PREFIX}:{}:{V3_MAPPACK_LB}:{}:{V3_MAPPACK_LB_WORST_RANK}",
        self.mappack_id, self.player_id
    )
}

create_key! {
    struct MappackPlayerRanks<'a => '_> = mappack_player_ranks_key {
        mappack_id: &'a str,
        player_id: u32,
    }
    |self, f| write!(
        f,
        "{V3_KEY_PREFIX}:{V3_MAPPACK_KEY_PREFIX}:{}:{V3_MAPPACK_LB}:{}:{V3_MAPPACK_LB_RANKS}",
        self.mappack_id, self.player_id
    )
}

create_key! {
    struct MappackMapLastRank<'a => '_> = mappack_map_last_rank {
        mappack_id: &'a str,
        game_id: &'a str,
    }
    |self, f| write!(
        f,
        "{V3_KEY_PREFIX}:{V3_MAPPACK_KEY_PREFIX}:{}:{V3_MAPPACK_MAP_KEY_PREFIX}:{}:{V3_MAPPACK_MAP_LAST_RANK}",
        self.mappack_id, self.game_id
    )
}

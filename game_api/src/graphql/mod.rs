use actix_session::Session;
use actix_web::web::{self, Data};
use actix_web::{HttpResponse, Resource, Responder};
use async_graphql::dataloader::DataLoader;
use async_graphql::extensions::ApolloTracing;
use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql::{connection, Enum, ErrorExtensionValues, Value, ID};
use async_graphql_actix_web::GraphQLRequest;
use futures::StreamExt;
use records_lib::models::{self, RecordAttr};
use records_lib::update_ranks::get_rank_or_full_update;
use records_lib::Database;
use reqwest::Client;
use sqlx::{mysql, query_as, FromRow, MySqlPool, Row};
use std::env::var;
use std::vec::Vec;
use tracing_actix_web::RequestId;

use crate::auth::{self, privilege, WebToken, WEB_TOKEN_SESS_KEY};
use crate::graphql::map::MapLoader;
use crate::graphql::player::PlayerLoader;

use self::ban::Banishment;
use self::event::{Event, EventCategoryLoader, EventLoader};
use self::map::Map;
use self::mappack::Mappack;
use self::player::Player;
use self::record::RankedRecord;
use self::utils::{
    connections_append_query_string, connections_bind_query_parameters, connections_pages_info,
    decode_id,
};

mod ban;
mod event;
mod map;
mod mappack;
mod medal;
mod player;
mod rating;
mod record;
mod utils;

#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Enum)]
pub(crate) enum SortState {
    Sort,
    Reverse,
}

impl SortState {
    pub fn sql_order_by(this: &Option<Self>) -> &'static str {
        match this.as_ref() {
            Some(Self::Reverse) => "ASC",
            _ => "DESC",
        }
    }
}

#[derive(async_graphql::Interface)]
#[graphql(field(name = "id", ty = "ID"))]
enum Node {
    Map(Map),
    Player(Player),
}

pub struct QueryRoot;

#[async_graphql::Object]
impl QueryRoot {
    async fn mappack(
        &self,
        ctx: &async_graphql::Context<'_>,
        mappack_id: String,
    ) -> async_graphql::Result<Mappack> {
        let res = mappack::calc_scores(ctx, mappack_id).await?;
        Ok(res)
    }

    async fn banishments(
        &self,
        ctx: &async_graphql::Context<'_>,
    ) -> async_graphql::Result<Vec<Banishment>> {
        let db = ctx.data_unchecked();
        let Some(WebToken { login, token }) = ctx.data_opt::<WebToken>() else {
            return Err(async_graphql::Error::new("Unauthorized"));
        };
        auth::website_check_auth_for(db, login, token, privilege::ADMIN).await?;
        Ok(query_as("SELECT * FROM banishments")
            .fetch_all(&db.mysql_pool)
            .await?)
    }

    async fn events(&self, ctx: &async_graphql::Context<'_>) -> async_graphql::Result<Vec<Event>> {
        let db = ctx.data_unchecked::<MySqlPool>();
        Ok(query_as("SELECT * FROM event").fetch_all(db).await?)
    }

    async fn players(
        &self,
        ctx: &async_graphql::Context<'_>,
        after: Option<String>,
        before: Option<String>,
        first: Option<i32>,
        last: Option<i32>,
    ) -> async_graphql::Result<connection::Connection<ID, Player>> {
        connection::query(
            after,
            before,
            first,
            last,
            |after: Option<ID>, before: Option<ID>, first: Option<usize>, last: Option<usize>| async move {
                let after = decode_id(after.as_ref());
                let before = decode_id(before.as_ref());

                // Build the query string
                let mut query = String::from("SELECT id, login, name FROM players ");
                connections_append_query_string(&mut query, false, after, before, first, last);
                let reversed = first.is_none() && last.is_some();

                // Bind the parameters
                let mut query = sqlx::query(&query);
                query = connections_bind_query_parameters(query, after, before, first, last);

                // Execute the query
                let mysql_pool = ctx.data_unchecked::<MySqlPool>();
                let mut players =
                    query
                        .map(|x: mysql::MySqlRow| {
                            let cursor = ID(format!("v0:Player:{}", x.get::<u32, _>(0)));
                            connection::Edge::new(cursor, Player::from_row(&x).unwrap())
                        })
                        .fetch_all(mysql_pool)
                        .await?;
                if reversed {
                    players.reverse();
                }

                let (has_previous_page, has_next_page) = connections_pages_info(players.len(), first, last);
                let mut connection = connection::Connection::new(has_previous_page, has_next_page);
                connection.edges.extend(players);

                Ok::<_, sqlx::Error>(connection)
            },
        )
        .await
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
                let time_start = std::time::SystemTime::now();

                let after  = decode_id(after.as_ref());
                let before = decode_id(before.as_ref());

                // Build the query string
                let mut query = String::from("SELECT id, game_id, player_id, name FROM maps ");
                connections_append_query_string(&mut query, false, after, before, first, last);

                println!("root maps: {:.3?}\n", time_start.elapsed().unwrap());

                let reversed = first.is_none() && last.is_some();

                // Bind the parameters
                let mut query = sqlx::query(&query);
                query = connections_bind_query_parameters(query, after, before, first, last);

                // Execute the query
                let mysql_pool = ctx.data_unchecked::<MySqlPool>();
                let mut maps =
                    query
                        .map(|x: mysql::MySqlRow| {
                            let cursor = ID(format!("v0:Map:{}", x.get::<u32, _>(0)));
                            connection::Edge::new(cursor, Map::from_row(&x).unwrap())
                        })
                        .fetch_all(mysql_pool)
                        .await?;

                println!("root maps: done await query {:.3?}", time_start.elapsed().unwrap());

                if reversed {
                    maps.reverse();
                }

                let (has_previous_page, has_next_page) = connections_pages_info(maps.len(), first, last);
                let mut connection = connection::Connection::new(has_previous_page, has_next_page);
                connection.edges.extend(maps);

                println!("root maps: done connection {:.3?}", time_start.elapsed().unwrap());
                Ok::<_, sqlx::Error>(connection)
            },
        )
        .await
    }

    // Global unique identifiers
    async fn node(&self, ctx: &async_graphql::Context<'_>, id: async_graphql::ID) -> Option<Node> {
        let mysql_pool = ctx.data_unchecked::<MySqlPool>();
        let parts: Vec<&str> = id.split(':').collect();
        println!("node");

        if parts.len() != 3 || parts[0] != "v0" || (parts[1] != "Map" && parts[1] != "Player") {
            println!(
                "invalid, len: {}, [0]: {}, [1]: {}",
                parts.len(),
                parts[0],
                parts[1]
            );
            None
        } else if parts[1] == "Map" {
            let id = parts[2].parse::<u32>().ok()?;
            let query = "SELECT id, game_id, player_id, name FROM maps WHERE id = ? ";
            println!("{} _ id: {}", &query, id);
            let query = sqlx::query(query).bind(id);
            let result = query.fetch_one(mysql_pool).await.ok()?;
            Some(Node::Map(Map::from_row(&result).unwrap()))
        } else {
            let id = parts[2].parse::<u32>().ok()?;
            let query = "SELECT id, login, name FROM players WHERE id = ? ";
            println!("{} _ id: {}", &query, id);
            let query = sqlx::query(query).bind(id);
            let result = query.fetch_one(mysql_pool).await.ok()?;
            Some(Node::Player(Player::from_row(&result).unwrap()))
        }
    }

    // Old website

    async fn record(
        &self,
        ctx: &async_graphql::Context<'_>,
        record_id: u32,
    ) -> async_graphql::Result<RankedRecord> {
        let db = ctx.data_unchecked::<Database>();

        let Some(row) = sqlx::query(
            "SELECT r.*, m.* FROM records r
            INNER JOIN maps m ON m.id = r.map_id
            WHERE r.record_id = ?",
        )
        .bind(record_id)
        .fetch_optional(&db.mysql_pool)
        .await?
        else {
            return Err(async_graphql::Error::new("Record not found."));
        };

        let (record, map) = (
            models::Record::from_row(&row)?,
            models::Map::from_row(&row)?,
        );

        let redis_conn = &mut db.redis_pool.get().await?;
        let mysql_conn = &mut db.mysql_pool.acquire().await?;

        Ok(models::RankedRecord {
            rank: get_rank_or_full_update((mysql_conn, redis_conn), &map, record.time, None)
                .await?,
            record,
        }
        .into())
    }

    async fn map(
        &self,
        ctx: &async_graphql::Context<'_>,
        game_id: String,
    ) -> async_graphql::Result<Map> {
        let db = ctx.data_unchecked::<Database>();
        records_lib::map::get_map_from_game_id(&db.mysql_pool, &game_id)
            .await?
            .ok_or_else(|| async_graphql::Error::new("Map not found."))
            .map(Into::into)
    }

    async fn player(
        &self,
        ctx: &async_graphql::Context<'_>,
        login: String,
    ) -> async_graphql::Result<Player> {
        let mysql_pool = ctx.data_unchecked::<MySqlPool>();
        let query =
            sqlx::query_as::<_, Player>("SELECT * FROM players WHERE login = ? ").bind(login);
        Ok(query.fetch_one(mysql_pool).await?)
    }

    async fn records(
        &self,
        ctx: &async_graphql::Context<'_>,
        date_sort_by: Option<SortState>,
    ) -> async_graphql::Result<Vec<RankedRecord>> {
        let db = ctx.data_unchecked::<Database>();
        let redis_conn = &mut db.redis_pool.get().await?;
        let mysql_conn = &mut db.mysql_pool.acquire().await?;

        let date_sort_by = SortState::sql_order_by(&date_sort_by);

        let query = format!(
            "SELECT * FROM global_records
            WHERE game_id NOT LIKE '%_benchmark'
            ORDER BY record_date {date_sort_by}
            LIMIT 100"
        );

        let mut records = sqlx::query_as::<_, RecordAttr>(&query).fetch(&mut **mysql_conn);
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

pub type Schema = async_graphql::Schema<
    QueryRoot,
    async_graphql::EmptyMutation,
    async_graphql::EmptySubscription,
>;

#[allow(clippy::let_and_return)]
fn create_schema(db: Database, client: Client) -> Schema {
    let schema = async_graphql::Schema::build(
        QueryRoot,
        async_graphql::EmptyMutation,
        async_graphql::EmptySubscription,
    )
    .extension(ApolloTracing)
    .data(DataLoader::new(
        PlayerLoader(db.mysql_pool.clone()),
        tokio::spawn,
    ))
    .data(DataLoader::new(
        MapLoader(db.mysql_pool.clone()),
        tokio::spawn,
    ))
    .data(DataLoader::new(
        EventLoader(db.mysql_pool.clone()),
        tokio::spawn,
    ))
    .data(DataLoader::new(
        EventCategoryLoader(db.mysql_pool.clone()),
        tokio::spawn,
    ))
    .data(db.mysql_pool.clone())
    .data(db.redis_pool.clone())
    .data(db)
    .data(client)
    .limit_depth(16)
    .finish();

    #[cfg(feature = "output_gql_schema")]
    {
        use std::fs::OpenOptions;
        use std::io::Write;
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(format!(
                "schemas/schema_{}.graphql",
                chrono::Utc::now().timestamp()
            ))
            .unwrap()
            .write_all(schema.sdl().as_bytes())
            .unwrap();
    }

    #[cfg(feature = "localhost_test")]
    {
        println!("----------- Schema:");
        println!("{}", &schema.sdl());
        println!("----------- End schema");
    }

    schema
}

async fn index_graphql(
    request_id: RequestId,
    session: Session,
    schema: Data<Schema>,
    GraphQLRequest(request): GraphQLRequest,
) -> impl Responder {
    let web_token = session
        .get::<WebToken>(WEB_TOKEN_SESS_KEY)
        .expect("unable to retrieve web token");

    let request = {
        if let Some(web_token) = web_token {
            request.data(web_token)
        } else {
            request
        }
    };

    let mut result = schema.execute(request).await;
    for error in &mut result.errors {
        let ex = error
            .extensions
            .get_or_insert_with(ErrorExtensionValues::default);

        ex.set("request_id", Value::String(request_id.to_string()));
    }

    web::Json(result)
}

async fn index_playground() -> impl Responder {
    let gql_endpoint = var("GQL_ENDPOINT").unwrap_or_else(|_| {
        println!("GQL_ENDPOINT env var not set, using /graphql as default");
        "/graphql".to_owned()
    });
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(playground_source(GraphQLPlaygroundConfig::new(
            &gql_endpoint,
        )))
}

pub fn graphql_route(db: Database, client: Client) -> Resource {
    web::resource("/graphql")
        .app_data(Data::new(create_schema(db, client)))
        .route(web::get().to(index_playground))
        .route(web::post().to(index_graphql))
}

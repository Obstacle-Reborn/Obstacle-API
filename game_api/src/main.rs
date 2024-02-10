use actix_cors::Cors;
use actix_session::{
    config::{CookieContentSecurity, PersistentSession},
    storage::CookieSessionStore,
    SessionMiddleware,
};
use actix_web::{
    cookie::{time::Duration as CookieDuration, Key},
    web::{self, Data},
    App, HttpServer, Responder,
};
use anyhow::Context;
#[cfg(not(feature = "localhost_test"))]
use records_lib::get_env_var;
use game_api::{
    api_route, get_tokens_ttl, graphql_route, AuthState, FitRequestId, RecordsErrorKind,
    RecordsResponse,
};
use records_lib::{get_env_var_as, get_mysql_pool, get_redis_pool, read_env_var_file, Database};
use reqwest::Client;
use std::env::var;
use tracing_actix_web::{RequestId, TracingLogger};
use tracing_subscriber::fmt::format::FmtSpan;

async fn not_found(req_id: RequestId) -> RecordsResponse<impl Responder> {
    Err::<String, _>(RecordsErrorKind::EndpointNotFound).fit(req_id)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv()?;

    let filter = var("RECORDS_API_LOG")
        .unwrap_or_else(|_| "tracing=info,warp=info,game_api=info".to_owned());

    let port = get_env_var_as("RECORDS_API_PORT");

    let mysql_pool = get_mysql_pool().await.context("Cannot create MySQL pool")?;
    let redis_pool = get_redis_pool().context("Cannot create Redis pool")?;

    let db = Database {
        mysql_pool,
        redis_pool,
    };

    let client = Client::new();

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_span_events(FmtSpan::CLOSE)
        .init();

    let auth_state = Data::new(AuthState::default());

    #[cfg(not(feature = "localhost_test"))]
    let localhost_origin = get_env_var("RECORDS_API_HOST");

    let sess_key = Key::from(read_env_var_file("RECORDS_API_SESSION_KEY_FILE").as_bytes());

    HttpServer::new(move || {
        let cors = Cors::default()
            .supports_credentials()
            .allowed_methods(vec!["GET", "POST"])
            .allowed_headers(vec!["accept", "content-type"])
            .max_age(3600);
        #[cfg(feature = "localhost_test")]
        let cors = cors.allow_any_origin();
        #[cfg(not(feature = "localhost_test"))]
        let cors = cors.allowed_origin(&localhost_origin);

        App::new()
            .wrap(cors)
            .wrap(TracingLogger::default())
            .wrap(
                SessionMiddleware::builder(CookieSessionStore::default(), sess_key.clone())
                    .cookie_secure(!cfg!(feature = "localhost_test"))
                    .cookie_content_security(CookieContentSecurity::Private)
                    .session_lifecycle(
                        PersistentSession::default()
                            .session_ttl(CookieDuration::seconds(get_tokens_ttl() as i64)),
                    )
                    .build(),
            )
            .app_data(auth_state.clone())
            .app_data(client.clone())
            .app_data(Data::new(db.clone()))
            .service(graphql_route(db.clone(), client.clone()))
            .service(api_route())
            .default_service(web::to(not_found))
    })
    .bind(("0.0.0.0", port))
    .context("Cannot bind 0.0.0.0 address")?
    .run()
    .await
    .context("Cannot create actix-web server")?;

    Ok(())
}

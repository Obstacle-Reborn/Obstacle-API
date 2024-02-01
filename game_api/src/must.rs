//! See [`records_lib::must`] module documentation.

use actix_web::{web::Data, HttpMessage, HttpRequest};
use records_lib::Database;
use tracing_actix_web::RequestId;

pub fn have_request_id(req: &HttpRequest) -> RequestId {
    *req.extensions()
        .get::<RequestId>()
        .expect("RequestId should be present")
}

pub fn have_db(req: &HttpRequest) -> Data<Database> {
    req.app_data::<Data<Database>>()
        .expect("Data<Database> app data should be present")
        .clone()
}

use async_graphql::ID;
use sqlx::mysql;

pub fn decode_id(id: Option<&ID>) -> Option<u32> {
    let parts: Vec<&str> = id?.split(':').collect();
    if parts.len() != 3 || parts[0] != "v0" || (parts[1] != "Map" && parts[1] != "Player") {
        println!(
            "invalid, len: {}, [0]: {}, [1]: {}",
            parts.len(),
            parts[0],
            parts[1]
        );
        None
    } else {
        parts[2].parse::<u32>().ok()
    }
}

pub fn connections_append_query_string_page(
    query: &mut String,
    has_where_clause: bool,
    after: Option<u32>,
    before: Option<u32>,
) {
    if before.is_some() || after.is_some() {
        query.push_str(if !has_where_clause { "WHERE " } else { "and " });

        match (before, after) {
            (Some(_), Some(_)) => query.push_str("id > ? and id < ? "), // after, before
            (Some(_), _) => query.push_str("id < ? "),                  // before
            (_, Some(_)) => query.push_str("id > ? "),                  // after
            _ => unreachable!(),
        }
    }
}

pub fn connections_append_query_string_order(
    query: &mut String,
    first: Option<usize>,
    last: Option<usize>,
) {
    if first.is_some() {
        query.push_str("ORDER BY id ASC LIMIT ? "); // first
    } else if last.is_some() {
        query.push_str("ORDER BY id DESC LIMIT ? "); // last
    }
}

pub fn connections_append_query_string(
    query: &mut String,
    has_where_clause: bool,
    after: Option<u32>,
    before: Option<u32>,
    first: Option<usize>,
    last: Option<usize>,
) {
    connections_append_query_string_page(query, has_where_clause, after, before);
    connections_append_query_string_order(query, first, last);
}

pub type SqlQuery<'q> = sqlx::query::Query<
    'q,
    mysql::MySql,
    <mysql::MySql as sqlx::database::HasArguments<'q>>::Arguments,
>;

pub fn connections_bind_query_parameters_page(
    mut query: SqlQuery,
    after: Option<u32>,
    before: Option<u32>,
) -> SqlQuery {
    match (before, after) {
        (Some(before), Some(after)) => query = query.bind(before).bind(after),
        (Some(before), _) => query = query.bind(before),
        (_, Some(after)) => query = query.bind(after),
        _ => {}
    }
    query
}

pub fn connections_bind_query_parameters_order(
    mut query: SqlQuery,
    first: Option<usize>,
    last: Option<usize>,
) -> SqlQuery {
    // Actual limits are N+1 to check if previous/next pages
    if let Some(first) = first {
        query = query.bind(first as u32 + 1);
    } else if let Some(last) = last {
        query = query.bind(last as u32 + 1);
    }

    query
}

pub fn connections_bind_query_parameters(
    mut query: SqlQuery,
    after: Option<u32>,
    before: Option<u32>,
    first: Option<usize>,
    last: Option<usize>,
) -> SqlQuery {
    query = connections_bind_query_parameters_page(query, after, before);
    query = connections_bind_query_parameters_order(query, first, last);
    query
}

pub fn connections_pages_info(
    results_count: usize,
    first: Option<usize>,
    last: Option<usize>,
) -> (bool, bool) {
    let mut has_previous_page = false;
    let mut has_next_page = false;

    if let Some(first) = first {
        if results_count == first + 1 {
            has_next_page = true;
        }
    }

    if let Some(last) = last {
        if results_count == last + 1 {
            has_previous_page = true;
        }
    }

    (has_previous_page, has_next_page)
}

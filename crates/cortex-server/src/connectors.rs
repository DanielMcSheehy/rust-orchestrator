//! External query engines behind `POST /api/query {"connector": ...}`.
//!
//! Three kinds:
//! - **postgres** — native protocol via `tokio-postgres` (read queries; rows
//!   are converted column-by-column to JSON)
//! - **clickhouse** — the HTTP interface; the SQL is sent with
//!   `FORMAT JSONEachRow` appended unless the query already names a format
//! - **chdb** — embedded ClickHouse, executed *inside the Python worker
//!   runtime* (requires `pip install chdb` in the worker environment). This
//!   reuses Cortex's own execution engine instead of linking libchdb into
//!   the server binary.

use cortex_core::{Connector, ConnectorKind, Runtime};
use cortex_executor::ExecRequest;
use serde_json::{json, Map, Value};

use crate::state::SharedState;

pub struct ConnectorResult {
    pub rows: Vec<Value>,
    pub truncated: bool,
}

pub async fn query(
    state: &SharedState,
    connector: &Connector,
    sql: &str,
    limit: usize,
) -> Result<ConnectorResult, String> {
    match connector.kind {
        ConnectorKind::Postgres => query_postgres(&connector.url, sql, limit).await,
        ConnectorKind::Clickhouse => query_clickhouse(&connector.url, sql, limit).await,
        ConnectorKind::Chdb => query_chdb(state, sql, limit).await,
    }
}

// ── postgres ─────────────────────────────────────────────────────────────

async fn query_postgres(url: &str, sql: &str, limit: usize) -> Result<ConnectorResult, String> {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .map_err(|e| format!("postgres connect failed: {e}"))?;
    let conn_task = tokio::spawn(connection);

    let result = client
        .query(sql, &[])
        .await
        .map_err(|e| format!("postgres query failed: {e}"));
    drop(client);
    conn_task.abort();
    let pg_rows = result?;

    let truncated = pg_rows.len() > limit;
    let rows = pg_rows
        .iter()
        .take(limit)
        .map(|row| {
            let mut obj = Map::new();
            for (i, col) in row.columns().iter().enumerate() {
                obj.insert(col.name().to_string(), pg_value(row, i));
            }
            Value::Object(obj)
        })
        .collect();
    Ok(ConnectorResult { rows, truncated })
}

/// Convert one Postgres column to JSON, covering the common wire types.
fn pg_value(row: &tokio_postgres::Row, i: usize) -> Value {
    use tokio_postgres::types::Type;
    let ty = row.columns()[i].type_();
    let get = |v: Option<Value>| v.unwrap_or(Value::Null);
    match *ty {
        Type::BOOL => get(row.try_get::<_, Option<bool>>(i).ok().flatten().map(Value::from)),
        Type::INT2 => get(row.try_get::<_, Option<i16>>(i).ok().flatten().map(Value::from)),
        Type::INT4 => get(row.try_get::<_, Option<i32>>(i).ok().flatten().map(Value::from)),
        Type::INT8 => get(row.try_get::<_, Option<i64>>(i).ok().flatten().map(Value::from)),
        Type::FLOAT4 => get(row.try_get::<_, Option<f32>>(i).ok().flatten().map(Value::from)),
        Type::FLOAT8 => get(row.try_get::<_, Option<f64>>(i).ok().flatten().map(Value::from)),
        Type::JSON | Type::JSONB => get(
            row.try_get::<_, Option<serde_json::Value>>(i).ok().flatten(),
        ),
        Type::TIMESTAMP => get(
            row.try_get::<_, Option<chrono::NaiveDateTime>>(i)
                .ok()
                .flatten()
                .map(|t| Value::from(t.to_string())),
        ),
        Type::TIMESTAMPTZ => get(
            row.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(i)
                .ok()
                .flatten()
                .map(|t| Value::from(t.to_rfc3339())),
        ),
        Type::DATE => get(
            row.try_get::<_, Option<chrono::NaiveDate>>(i)
                .ok()
                .flatten()
                .map(|d| Value::from(d.to_string())),
        ),
        Type::UUID => get(
            row.try_get::<_, Option<uuid::Uuid>>(i)
                .ok()
                .flatten()
                .map(|u| Value::from(u.to_string())),
        ),
        // TEXT, VARCHAR, NAME, NUMERIC-as-text fallback, everything else.
        _ => get(
            row.try_get::<_, Option<String>>(i)
                .ok()
                .flatten()
                .map(Value::from),
        ),
    }
}

// ── clickhouse (HTTP interface) ──────────────────────────────────────────

async fn query_clickhouse(url: &str, sql: &str, limit: usize) -> Result<ConnectorResult, String> {
    let sql_upper = sql.to_uppercase();
    let body = if sql_upper.contains("FORMAT ") {
        sql.to_string()
    } else {
        format!("{sql} FORMAT JSONEachRow")
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(url)
        .body(body)
        .send()
        .await
        .map_err(|e| format!("clickhouse request failed: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("clickhouse error ({status}): {}", text.trim()));
    }
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if rows.len() > limit {
            break;
        }
        rows.push(
            serde_json::from_str(line)
                .map_err(|e| format!("clickhouse returned non-JSONEachRow output: {e}"))?,
        );
    }
    let truncated = rows.len() > limit;
    rows.truncate(limit);
    Ok(ConnectorResult { rows, truncated })
}

// ── chdb (embedded ClickHouse in the Python worker) ──────────────────────

const CHDB_JOB: &str = r#"
def handler(params, inputs):
    try:
        import chdb
    except ImportError:
        raise RuntimeError(
            "chdb is not installed in the Python worker environment - pip install chdb"
        )
    import json
    out = str(chdb.query(params["sql"], "JSONEachRow"))
    rows = [json.loads(line) for line in out.splitlines() if line.strip()]
    cap = params["cap"]
    return {"rows": rows[:cap], "truncated": len(rows) > cap}
"#;

async fn query_chdb(
    state: &SharedState,
    sql: &str,
    limit: usize,
) -> Result<ConnectorResult, String> {
    let outcome = state
        .executor
        .execute(
            ExecRequest {
                runtime: Runtime::Python,
                code: CHDB_JOB.to_string(),
                params: json!({ "sql": sql, "cap": limit }),
                inputs: Value::Null,
                timeout_secs: 300,
            },
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
    let rows = outcome.value["rows"]
        .as_array()
        .cloned()
        .ok_or("chdb returned no rows array")?;
    Ok(ConnectorResult {
        rows,
        truncated: outcome.value["truncated"].as_bool().unwrap_or(false),
    })
}

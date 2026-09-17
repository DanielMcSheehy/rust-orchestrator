//! SQL over ingested datasets, powered by Polars — a Rust dataframe engine.
//!
//! Every dataset (NDJSON on disk) is registered as a SQL table under its own
//! name and a `-`→`_` alias, so `SELECT sensor, avg(value) FROM sensor_readings
//! GROUP BY sensor` works across datasets, including joins. Scans are lazy:
//! Polars pushes projections and predicates down to the file scan, so queries
//! only materialize what they select.

use std::path::Path;

use polars::prelude::*;
use polars::sql::SQLContext;

#[derive(Debug)]
pub struct QueryOutcome {
    /// JSON array of row objects, already serialized by Polars.
    pub rows_json: Vec<u8>,
    pub row_count: usize,
    pub truncated: bool,
}

/// Execute one SQL query against the registered datasets. Blocking —
/// call via `spawn_blocking`.
pub fn run_query(
    datasets_dir: &Path,
    datasets: &[String],
    sql: &str,
    limit: usize,
) -> Result<QueryOutcome, String> {
    let mut ctx = SQLContext::new();
    for name in datasets {
        let path = datasets_dir.join(format!("{name}.ndjson"));
        if !path.exists() {
            continue;
        }
        let lf = LazyJsonLineReader::new(PlPath::new(&path.to_string_lossy()))
            .with_infer_schema_length(Some(1000.try_into().expect("nonzero")))
            .finish()
            .map_err(|e| format!("failed to scan dataset `{name}`: {e}"))?;
        ctx.register(name, lf.clone());
        let alias = name.replace('-', "_");
        if alias != *name {
            ctx.register(&alias, lf);
        }
    }

    let lazy = ctx.execute(sql).map_err(|e| format!("query failed: {e}"))?;
    // Fetch one row beyond the cap so truncation is detectable.
    let mut df = lazy
        .limit((limit + 1) as IdxSize)
        .collect()
        .map_err(|e| format!("query failed: {e}"))?;
    let truncated = df.height() > limit;
    if truncated {
        df = df.head(Some(limit));
    }

    let mut rows_json = Vec::new();
    JsonWriter::new(&mut rows_json)
        .with_json_format(JsonFormat::Json)
        .finish(&mut df)
        .map_err(|e| format!("failed to serialize result: {e}"))?;
    Ok(QueryOutcome {
        rows_json,
        row_count: df.height(),
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_dataset(dir: &Path, name: &str, lines: &[&str]) {
        std::fs::write(dir.join(format!("{name}.ndjson")), lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn aggregates_with_sql() {
        let dir = tempfile::tempdir().unwrap();
        write_dataset(
            dir.path(),
            "readings",
            &[
                r#"{"sensor":"a","value":10.0}"#,
                r#"{"sensor":"a","value":20.0}"#,
                r#"{"sensor":"b","value":5.0}"#,
            ],
        );
        let out = run_query(
            dir.path(),
            &["readings".into()],
            "SELECT sensor, AVG(value) AS avg_value, COUNT(*) AS n \
             FROM readings GROUP BY sensor ORDER BY sensor",
            100,
        )
        .unwrap();
        let rows: serde_json::Value = serde_json::from_slice(&out.rows_json).unwrap();
        assert_eq!(out.row_count, 2);
        assert!(!out.truncated);
        assert_eq!(rows[0]["sensor"], "a");
        assert_eq!(rows[0]["avg_value"], 15.0);
        assert_eq!(rows[1]["n"], 1);
    }

    #[test]
    fn dashed_names_get_underscore_alias() {
        let dir = tempfile::tempdir().unwrap();
        write_dataset(dir.path(), "sensor-readings", &[r#"{"v":1}"#, r#"{"v":2}"#]);
        let out = run_query(
            dir.path(),
            &["sensor-readings".into()],
            "SELECT SUM(v) AS total FROM sensor_readings",
            100,
        )
        .unwrap();
        let rows: serde_json::Value = serde_json::from_slice(&out.rows_json).unwrap();
        assert_eq!(rows[0]["total"], 3);
    }

    #[test]
    fn truncates_at_limit() {
        let dir = tempfile::tempdir().unwrap();
        write_dataset(dir.path(), "big", &[r#"{"v":1}"#, r#"{"v":2}"#, r#"{"v":3}"#]);
        let out = run_query(dir.path(), &["big".into()], "SELECT * FROM big", 2).unwrap();
        assert_eq!(out.row_count, 2);
        assert!(out.truncated);
    }

    #[test]
    fn bad_sql_is_a_client_error() {
        let dir = tempfile::tempdir().unwrap();
        write_dataset(dir.path(), "d", &[r#"{"v":1}"#]);
        let err = run_query(dir.path(), &["d".into()], "SELEKT nope", 10).unwrap_err();
        assert!(err.contains("query failed"));
    }
}

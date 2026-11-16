//! SQL sandbox: the single security boundary through which any Ask SQL runs.
//!
//! Opens the Ask projection read-only, validates the statement statically and
//! via SQLite's authorizer allow-list, enforces limits and wall-clock timeout,
//! and returns typed columns and rows or a structured rejection with a rule code.
use crate::ask::projection;
use rusqlite::{
    Connection, OpenFlags,
    config::DbConfig,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    types::ValueRef,
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use ts_rs::TS;

pub const MAX_SQL_BYTES: usize = 8 * 1024;
pub const DEFAULT_ROW_LIMIT: usize = 1000;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug)]
pub struct SandboxLimits {
    pub timeout: Duration,
    pub max_rows: usize,
    pub max_sql_bytes: usize,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            max_rows: DEFAULT_ROW_LIMIT,
            max_sql_bytes: MAX_SQL_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SandboxRule {
    EmptySql,
    SqlTooLong,
    NulByte,
    NotSelect,
    MultipleStatements,
    NonReadOnly,
    UnauthorizedAction,
    UnauthorizedTable,
    UnauthorizedColumn,
    Timeout,
    BlobNotAllowed,
    SqlError,
    ProjectionUnavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
pub enum SqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SandboxOutput {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<SqlValue>>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxRejection {
    pub rule: SandboxRule,
    pub message: String,
    pub detail: String,
}

impl SandboxRejection {
    fn new(rule: SandboxRule, message: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            rule,
            message: message.into(),
            detail: detail.into(),
        }
    }
}

const ALLOWED_FUNCTIONS: &[&str] = &[
    // Aggregates
    "avg",
    "count",
    "group_concat",
    "max",
    "min",
    "string_agg",
    "sum",
    "total",
    // Window functions
    "cume_dist",
    "dense_rank",
    "first_value",
    "lag",
    "last_value",
    "lead",
    "nth_value",
    "ntile",
    "percent_rank",
    "rank",
    "row_number",
    // Core scalar & string
    "abs",
    "char",
    "coalesce",
    "concat",
    "concat_ws",
    "format",
    "glob",
    "hex",
    "ifnull",
    "iif",
    "instr",
    "length",
    "like",
    "lower",
    "ltrim",
    "nullif",
    "octet_length",
    "printf",
    "replace",
    "round",
    "rtrim",
    "sign",
    "substr",
    "substring",
    "trim",
    "typeof",
    "unicode",
    "upper",
    // Math
    "acos",
    "acosh",
    "asin",
    "asinh",
    "atan",
    "atan2",
    "atanh",
    "ceil",
    "ceiling",
    "cos",
    "cosh",
    "degrees",
    "exp",
    "floor",
    "ln",
    "log",
    "log10",
    "log2",
    "mod",
    "pi",
    "pow",
    "power",
    "radians",
    "sin",
    "sinh",
    "sqrt",
    "tan",
    "tanh",
    "trunc",
    // Date and time
    "date",
    "datetime",
    "julianday",
    "strftime",
    "time",
    "timediff",
    "unixepoch",
    // JSON read helpers
    "json",
    "json_array_length",
    "json_each",
    "json_extract",
    "json_tree",
    "json_type",
    "json_valid",
];

fn is_allowed_function(name: &str) -> bool {
    ALLOWED_FUNCTIONS
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(name))
}

/// Skip leading whitespace and SQL comments (`-- ...` and `/* ... */`) and
/// verify the statement starts with `SELECT` or `WITH`.
fn static_check(sql: &str, max_bytes: usize) -> Result<&str, SandboxRejection> {
    if sql.contains('\0') {
        return Err(SandboxRejection::new(
            SandboxRule::NulByte,
            "The SQL query cannot contain NUL bytes.",
            "nul_byte",
        ));
    }
    if sql.len() > max_bytes {
        return Err(SandboxRejection::new(
            SandboxRule::SqlTooLong,
            format!("The SQL query exceeds the {max_bytes}-byte limit."),
            "sql_too_long",
        ));
    }
    let trimmed = skip_leading_ws_and_comments(sql);
    if trimmed.is_empty() {
        return Err(SandboxRejection::new(
            SandboxRule::EmptySql,
            "The SQL query is empty.",
            "empty_sql",
        ));
    }
    if !starts_with_keyword(trimmed, "SELECT") && !starts_with_keyword(trimmed, "WITH") {
        return Err(SandboxRejection::new(
            SandboxRule::NotSelect,
            "Only SELECT or WITH queries are allowed.",
            "not_select",
        ));
    }
    if has_multiple_statements(trimmed) {
        return Err(SandboxRejection::new(
            SandboxRule::MultipleStatements,
            "Only a single SQL statement is allowed (rule `multiple_statements`).",
            "multiple_statements",
        ));
    }
    Ok(trimmed)
}

/// Detect any `;` followed by non-whitespace/non-comment tokens outside SQL
/// string/identifier literals and comments.
fn has_multiple_statements(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut saw_semi = false;
    while i < bytes.len() {
        if saw_semi {
            let rest = skip_leading_ws_and_comments(&sql[i..]);
            return !rest.is_empty();
        }
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            b'\'' | b'"' | b'`' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == quote {
                        if bytes.get(i + 1) == Some(&quote) {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'[' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b']' {
                    i += 1;
                }
                i = (i + 1).min(bytes.len());
            }
            b';' => {
                saw_semi = true;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    false
}

fn skip_leading_ws_and_comments(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if let Some(rest) = s.strip_prefix("--") {
            match rest.find('\n') {
                Some(idx) => s = &rest[idx + 1..],
                None => return "",
            }
        } else if let Some(rest) = s.strip_prefix("/*") {
            match rest.find("*/") {
                Some(idx) => s = &rest[idx + 2..],
                None => return s,
            }
        } else {
            return s;
        }
    }
}

fn starts_with_keyword(s: &str, keyword: &str) -> bool {
    let Some(prefix) = s.get(..keyword.len()) else {
        return false;
    };
    if !prefix.eq_ignore_ascii_case(keyword) {
        return false;
    }
    match s.as_bytes().get(keyword.len()) {
        None => true,
        Some(&b) => !(b.is_ascii_alphanumeric() || b == b'_'),
    }
}

fn configure_connection(conn: &Connection, limits: SandboxLimits) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA query_only = ON;\
         PRAGMA trusted_schema = OFF;\
         PRAGMA temp_store = MEMORY;\
         PRAGMA cache_size = -2000;",
    )?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DML, false)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DDL, false)?;

    conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;
    conn.set_limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 0)?;
    conn.set_limit(
        Limit::SQLITE_LIMIT_SQL_LENGTH,
        i32::try_from(limits.max_sql_bytes).unwrap_or(8192),
    )?;
    conn.set_limit(Limit::SQLITE_LIMIT_EXPR_DEPTH, 64)?;
    conn.set_limit(Limit::SQLITE_LIMIT_COMPOUND_SELECT, 16)?;
    conn.set_limit(Limit::SQLITE_LIMIT_FUNCTION_ARG, 16)?;
    conn.set_limit(Limit::SQLITE_LIMIT_LIKE_PATTERN_LENGTH, 256)?;
    conn.set_limit(Limit::SQLITE_LIMIT_COLUMN, 64)?;
    conn.set_limit(Limit::SQLITE_LIMIT_LENGTH, 1_048_576)?;
    Ok(())
}

type DeniedState = Arc<Mutex<Option<(SandboxRule, String)>>>;

fn authorize(ctx: AuthContext<'_>, denied: &DeniedState) -> Authorization {
    let record = |rule: SandboxRule, detail: String| {
        let mut slot = denied.lock().unwrap();
        if slot.is_none() {
            *slot = Some((rule, detail));
        }
        Authorization::Deny
    };

    match ctx.action {
        AuthAction::Select | AuthAction::Recursive => Authorization::Allow,
        AuthAction::Read {
            table_name,
            column_name,
        } => {
            if ctx.database_name.is_none()
                && !table_name.to_ascii_lowercase().starts_with("sqlite_")
                && !table_name.to_ascii_lowercase().starts_with("pragma_")
                && !table_name.eq_ignore_ascii_case("_projection_meta")
            {
                // Transient CTEs (such as recursive CTE self-references) and
                // eponymous functions (json_each/json_tree) have no backing DB.
                if table_name.eq_ignore_ascii_case("json_each")
                    || table_name.eq_ignore_ascii_case("json_tree")
                    || column_name.is_empty()
                {
                    return Authorization::Allow;
                }
            }
            if let Some(db) = ctx.database_name
                && !db.eq_ignore_ascii_case("main")
            {
                return record(
                    SandboxRule::UnauthorizedTable,
                    format!("table {db}.{table_name} is not in the Ask projection"),
                );
            }
            if !projection::is_allowed_table(table_name) {
                return record(
                    SandboxRule::UnauthorizedTable,
                    format!("table {table_name} is not in the Ask projection"),
                );
            }
            if !projection::is_allowed_column(table_name, column_name) {
                return record(
                    SandboxRule::UnauthorizedColumn,
                    format!("column {table_name}.{column_name} is not in the Ask projection"),
                );
            }
            Authorization::Allow
        }
        AuthAction::Function { function_name } => {
            if is_allowed_function(function_name) {
                Authorization::Allow
            } else {
                record(
                    SandboxRule::UnauthorizedAction,
                    format!("function {function_name} is not allowed"),
                )
            }
        }
        other => record(
            SandboxRule::UnauthorizedAction,
            format!("action {other:?} is not allowed"),
        ),
    }
}

fn map_sqlite_error(error: rusqlite::Error, denied: &DeniedState) -> SandboxRejection {
    if let Some((rule, detail)) = denied.lock().unwrap().take() {
        let message = format!("Query rejected by rule `{}`: {detail}.", rule_name(rule));
        return SandboxRejection::new(rule, message, detail);
    }
    match error {
        rusqlite::Error::MultipleStatement => SandboxRejection::new(
            SandboxRule::MultipleStatements,
            "Only a single SQL statement is allowed (rule `multiple_statements`).",
            "multiple_statements",
        ),
        rusqlite::Error::SqliteFailure(err, msg) => {
            if err.code == rusqlite::ErrorCode::OperationInterrupted {
                return SandboxRejection::new(
                    SandboxRule::Timeout,
                    "The SQL query exceeded the time limit and was stopped (rule `timeout`).",
                    "timeout",
                );
            }
            if err.code == rusqlite::ErrorCode::AuthorizationForStatementDenied {
                return SandboxRejection::new(
                    SandboxRule::UnauthorizedAction,
                    "The SQL query was denied by the sandbox authorizer (rule `unauthorized_action`).",
                    msg.unwrap_or_else(|| "unauthorized".into()),
                );
            }
            let detail = msg.unwrap_or_else(|| err.to_string());
            if let Some(table) = detail.strip_prefix("no such table: ") {
                return SandboxRejection::new(
                    SandboxRule::UnauthorizedTable,
                    format!(
                        "Query rejected by rule `{}`: table {table} is not in the Ask projection.",
                        rule_name(SandboxRule::UnauthorizedTable)
                    ),
                    detail,
                );
            }
            SandboxRejection::new(
                SandboxRule::SqlError,
                format!("The SQL query could not run: {detail}."),
                detail,
            )
        }
        other => {
            let detail = other.to_string();
            SandboxRejection::new(
                SandboxRule::SqlError,
                format!("The SQL query could not run: {detail}."),
                detail,
            )
        }
    }
}

fn rule_name(rule: SandboxRule) -> &'static str {
    match rule {
        SandboxRule::EmptySql => "empty_sql",
        SandboxRule::SqlTooLong => "sql_too_long",
        SandboxRule::NulByte => "nul_byte",
        SandboxRule::NotSelect => "not_select",
        SandboxRule::MultipleStatements => "multiple_statements",
        SandboxRule::NonReadOnly => "non_read_only",
        SandboxRule::UnauthorizedAction => "unauthorized_action",
        SandboxRule::UnauthorizedTable => "unauthorized_table",
        SandboxRule::UnauthorizedColumn => "unauthorized_column",
        SandboxRule::Timeout => "timeout",
        SandboxRule::BlobNotAllowed => "blob_not_allowed",
        SandboxRule::SqlError => "sql_error",
        SandboxRule::ProjectionUnavailable => "projection_unavailable",
    }
}

/// Run a single read-only SQL query against the Ask projection file inside the
/// sandbox. This is the only entry point for Ask SQL execution.
pub fn execute(
    projection_path: &Path,
    sql: &str,
    limits: SandboxLimits,
) -> std::result::Result<SandboxOutput, SandboxRejection> {
    static_check(sql, limits.max_sql_bytes)?;

    if !projection_path.is_file() {
        return Err(SandboxRejection::new(
            SandboxRule::ProjectionUnavailable,
            "The Ask projection is not built yet.",
            "projection_missing",
        ));
    }

    let conn = Connection::open_with_flags(
        projection_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| {
        SandboxRejection::new(
            SandboxRule::ProjectionUnavailable,
            "Could not open the Ask projection.",
            err.to_string(),
        )
    })?;

    configure_connection(&conn, limits).map_err(|err| {
        SandboxRejection::new(
            SandboxRule::SqlError,
            "Failed to configure the SQL sandbox.",
            err.to_string(),
        )
    })?;

    let denied: DeniedState = Arc::new(Mutex::new(None));
    let auth_denied = Arc::clone(&denied);
    conn.authorizer(Some(move |ctx: AuthContext<'_>| {
        authorize(ctx, &auth_denied)
    }))
    .map_err(|err| map_sqlite_error(err, &denied))?;

    let deadline = Instant::now() + limits.timeout;
    conn.progress_handler(1000, Some(move || Instant::now() >= deadline))
        .map_err(|err| {
            SandboxRejection::new(
                SandboxRule::SqlError,
                "Failed to configure the SQL timeout handler.",
                err.to_string(),
            )
        })?;

    let mut stmt = conn
        .prepare(sql)
        .map_err(|err| map_sqlite_error(err, &denied))?;

    if !stmt.readonly() {
        return Err(SandboxRejection::new(
            SandboxRule::NonReadOnly,
            "Only read-only statements are allowed (rule `non_read_only`).",
            "non_read_only",
        ));
    }

    let columns: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
    let col_count = columns.len();

    let mut query_rows = stmt
        .query([])
        .map_err(|err| map_sqlite_error(err, &denied))?;

    let mut rows = Vec::new();
    let mut truncated = false;
    while let Some(row) = query_rows
        .next()
        .map_err(|err| map_sqlite_error(err, &denied))?
    {
        if rows.len() >= limits.max_rows {
            truncated = true;
            break;
        }
        let mut values = Vec::with_capacity(col_count);
        for index in 0..col_count {
            let cell = row
                .get_ref(index)
                .map_err(|err| map_sqlite_error(err, &denied))?;
            let value = match cell {
                ValueRef::Null => SqlValue::Null,
                ValueRef::Integer(i) => SqlValue::Integer(i),
                ValueRef::Real(f) => {
                    if f.is_finite() {
                        SqlValue::Real(f)
                    } else {
                        SqlValue::Null
                    }
                }
                ValueRef::Text(bytes) => {
                    let text = std::str::from_utf8(bytes).map_err(|_| {
                        SandboxRejection::new(
                            SandboxRule::SqlError,
                            "Query returned invalid UTF-8 text.",
                            "invalid_utf8",
                        )
                    })?;
                    SqlValue::Text(text.to_owned())
                }
                ValueRef::Blob(_) => {
                    return Err(SandboxRejection::new(
                        SandboxRule::BlobNotAllowed,
                        "Binary BLOB values are not allowed in Ask results (rule `blob_not_allowed`).",
                        "blob_not_allowed",
                    ));
                }
            };
            values.push(value);
        }
        rows.push(values);
    }

    Ok(SandboxOutput {
        columns,
        rows,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ask::projection;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    fn fixture_projection() -> NamedTempFile {
        let file = NamedTempFile::new().unwrap();
        let conn = Connection::open(file.path()).unwrap();
        conn.execute_batch(projection::DDL).unwrap();
        conn.execute(
            "INSERT INTO artists(id, name) VALUES ('a1', 'Björk'), ('a2', 'Ado')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO albums(id, name) VALUES ('al1', 'Homogenic')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tracks(id, title, album_id, duration_ms) VALUES ('t1', 'Jóga', 'al1', 305000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO track_artists(track_id, artist_id, ordinal) VALUES ('t1', 'a1', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO listens(id, track_id, played_at_ms, local_date, week_date, local_year, local_month, weekday, local_hour, listened_ms, duration_kind, counted, needs_review) VALUES ('l1', 't1', 1704880800000, '2024-01-10', '2024-01-08', 2024, 1, 3, 11, 305000, 'actual', 1, 0)",
            [],
        )
        .unwrap();
        file
    }

    #[test]
    fn valid_select_and_cte_return_typed_columns_and_rows() {
        let proj = fixture_projection();
        let out = execute(
            proj.path(),
            "WITH counts AS (SELECT t.id, COUNT(*) AS plays FROM listens l JOIN tracks t ON t.id = l.track_id WHERE l.counted = 1 GROUP BY t.id) SELECT a.name AS artist, c.plays FROM counts c JOIN track_artists ta ON ta.track_id = c.id JOIN artists a ON a.id = ta.artist_id",
            SandboxLimits::default(),
        )
        .expect("valid CTE query should succeed");
        assert_eq!(out.columns, vec!["artist", "plays"]);
        assert_eq!(
            out.rows,
            vec![vec![SqlValue::Text("Björk".into()), SqlValue::Integer(1)]]
        );
        assert!(!out.truncated);
    }

    #[test]
    fn non_select_statements_are_rejected_with_rule_code() {
        let proj = fixture_projection();
        for sql in [
            "INSERT INTO artists(id, name) VALUES ('x', 'y')",
            "UPDATE artists SET name = 'x'",
            "DELETE FROM listens",
            "DROP TABLE listens",
            "PRAGMA table_info(listens)",
            "ATTACH DATABASE ':memory:' AS other",
            "EXPLAIN SELECT COUNT(*) FROM listens",
            "VACUUM",
        ] {
            let err = execute(proj.path(), sql, SandboxLimits::default())
                .expect_err(&format!("expected rejection for: {sql}"));
            assert_eq!(err.rule, SandboxRule::NotSelect, "{sql}: {err:?}");
        }
    }

    #[test]
    fn multiple_statements_are_rejected_with_rule_code() {
        let proj = fixture_projection();
        for sql in [
            "SELECT 1; SELECT 2",
            "SELECT COUNT(*) FROM listens; DROP TABLE listens",
            "SELECT 1; -- comment\nINSERT INTO artists VALUES ('x', 'y')",
        ] {
            let err = execute(proj.path(), sql, SandboxLimits::default())
                .expect_err(&format!("expected rejection for: {sql}"));
            assert_eq!(err.rule, SandboxRule::MultipleStatements, "{sql}: {err:?}");
        }
    }

    #[test]
    fn archive_only_and_system_tables_are_rejected() {
        let proj = fixture_projection();
        for sql in [
            "SELECT value FROM settings",
            "SELECT digest FROM sessions",
            "SELECT id FROM api_keys",
            "SELECT name FROM sqlite_master",
            "SELECT name FROM sqlite_schema",
            "SELECT revision FROM _projection_meta",
            "SELECT * FROM pragma_table_info('listens')",
        ] {
            let err = execute(proj.path(), sql, SandboxLimits::default())
                .expect_err(&format!("expected rejection for: {sql}"));
            assert_eq!(err.rule, SandboxRule::UnauthorizedTable, "{sql}: {err:?}");
        }
    }

    #[test]
    fn unauthorized_functions_and_columns_are_rejected() {
        let proj = fixture_projection();
        let err = execute(
            proj.path(),
            "SELECT sqlite_version()",
            SandboxLimits::default(),
        )
        .unwrap_err();
        assert_eq!(err.rule, SandboxRule::UnauthorizedAction);

        let err = execute(
            proj.path(),
            "SELECT load_extension('evil')",
            SandboxLimits::default(),
        )
        .unwrap_err();
        assert_eq!(err.rule, SandboxRule::UnauthorizedAction);

        let err = execute(
            proj.path(),
            "SELECT randomblob(100)",
            SandboxLimits::default(),
        )
        .unwrap_err();
        assert_eq!(err.rule, SandboxRule::UnauthorizedAction);

        let err = execute(
            proj.path(),
            "SELECT nonexistent_col FROM listens",
            SandboxLimits::default(),
        )
        .unwrap_err();
        assert!(
            matches!(
                err.rule,
                SandboxRule::UnauthorizedColumn | SandboxRule::SqlError
            ),
            "{err:?}"
        );
    }

    #[test]
    fn query_exceeding_time_budget_is_stopped_and_reported() {
        let proj = fixture_projection();
        let limits = SandboxLimits {
            timeout: Duration::from_millis(20),
            ..SandboxLimits::default()
        };
        let err = execute(
            proj.path(),
            "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt) SELECT COUNT(*) FROM cnt",
            limits,
        )
        .expect_err("infinite recursive CTE must time out");
        assert_eq!(err.rule, SandboxRule::Timeout, "{err:?}");
    }

    #[test]
    fn row_cap_truncates_and_marks_truncated() {
        let proj = fixture_projection();
        let limits = SandboxLimits {
            max_rows: 1,
            ..SandboxLimits::default()
        };
        let out = execute(
            proj.path(),
            "SELECT id, name FROM artists ORDER BY id",
            limits,
        )
        .unwrap();
        assert_eq!(out.rows.len(), 1);
        assert!(out.truncated);
    }
}

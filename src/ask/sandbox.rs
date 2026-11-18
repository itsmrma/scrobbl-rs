//! SQL sandbox: the single security boundary through which any Ask SQL runs.
//!
//! Opens the Ask projection read-only, validates the statement statically and
//! via SQLite's authorizer allow-list, enforces limits and wall-clock timeout,
//! and returns typed columns and rows or a structured rejection with a rule code.
use crate::ask::projection;
use rusqlite::{
    Connection, InterruptHandle, OpenFlags,
    config::DbConfig,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    types::ValueRef,
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use ts_rs::TS;

pub const MAX_SQL_BYTES: usize = 8 * 1024;
pub const DEFAULT_ROW_LIMIT: usize = 1000;
pub const DEFAULT_OUTPUT_BYTES: usize = 1024 * 1024;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);
pub const DEFAULT_INSTRUCTION_BUDGET: u64 = 5_000_000;
const PROGRESS_STEP_INTERVAL: i32 = 1000;

#[derive(Clone, Copy, Debug)]
pub struct SandboxLimits {
    pub timeout: Duration,
    pub max_rows: usize,
    pub max_bytes: usize,
    pub max_sql_bytes: usize,
    pub max_instructions: u64,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            max_rows: DEFAULT_ROW_LIMIT,
            max_bytes: DEFAULT_OUTPUT_BYTES,
            max_sql_bytes: MAX_SQL_BYTES,
            max_instructions: DEFAULT_INSTRUCTION_BUDGET,
        }
    }
}

/// Cancellation handle shared with an in-flight sandbox execution. Calling
/// [`SandboxCancel::cancel`] flips the cancellation flag and immediately
/// interrupts the underlying SQLite connection.
#[derive(Clone, Default)]
pub struct SandboxCancel {
    cancelled: Arc<AtomicBool>,
    interrupt: Arc<Mutex<Option<InterruptHandle>>>,
}

impl std::fmt::Debug for SandboxCancel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxCancel")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl SandboxCancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_flag(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            interrupt: Arc::new(Mutex::new(None)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(handle) = self
            .interrupt
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
        {
            handle.interrupt();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

struct InterruptRegistration<'a>(&'a SandboxCancel);

impl<'a> InterruptRegistration<'a> {
    fn register(cancel: &'a SandboxCancel, handle: InterruptHandle) -> Self {
        {
            let mut slot = cancel
                .interrupt
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            *slot = Some(handle);
        }
        if cancel.is_cancelled() {
            cancel.cancel();
        }
        Self(cancel)
    }
}

impl Drop for InterruptRegistration<'_> {
    fn drop(&mut self) {
        let mut slot = self
            .0
            .interrupt
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *slot = None;
    }
}

/// Declare the stable rule codes once: serde and [`SandboxRule::rule_name`] read
/// the same wire strings, so a message can never name a different code than the
/// one the browser receives.
macro_rules! sandbox_rules {
    ($($variant:ident => $wire:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
        pub enum SandboxRule {
            $(#[serde(rename = $wire)] $variant,)+
        }

        impl SandboxRule {
            /// The wire name serde uses for this rule, and the name used in
            /// owner-facing messages.
            pub fn rule_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire,)+
                }
            }

            /// True when the model can plausibly fix the query from the
            /// sanitized error. Cancellation, a timeout and a missing
            /// projection cannot be repaired by rewriting the SQL.
            pub fn is_repairable(self) -> bool {
                !matches!(
                    self,
                    Self::Cancelled | Self::Timeout | Self::ProjectionUnavailable
                )
            }
        }
    };
}

sandbox_rules! {
    EmptySql => "empty_sql",
    SqlTooLong => "sql_too_long",
    NulByte => "nul_byte",
    NotSelect => "not_select",
    MultipleStatements => "multiple_statements",
    BoundParameters => "bound_parameters",
    NonReadOnly => "non_read_only",
    UnauthorizedAction => "unauthorized_action",
    UnauthorizedTable => "unauthorized_table",
    UnauthorizedColumn => "unauthorized_column",
    Timeout => "timeout",
    InstructionLimit => "instruction_limit",
    Cancelled => "cancelled",
    BlobNotAllowed => "blob_not_allowed",
    OutputTooLarge => "output_too_large",
    SqlError => "sql_error",
    ProjectionUnavailable => "projection_unavailable",
}

/// One typed SQL cell. Serialization is untagged, so the browser receives a JSON
/// null, number or string.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
}

/// The browser contract for [`SqlValue`]. TypeScript cannot tell `Integer` and
/// `Real` apart, so it sees one `number`; the Rust variants stay separate so an
/// integral real never round-trips as an integer. ts-rs cannot spell this on an
/// untagged enum (`type`/`as` are rejected there), hence the explicit impl.
impl TS for SqlValue {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;

    fn name(_: &ts_rs::Config) -> String {
        "SqlValue".to_owned()
    }

    fn inline(_: &ts_rs::Config) -> String {
        "null | number | string".to_owned()
    }

    fn decl(cfg: &ts_rs::Config) -> String {
        format!("type {} = {};", Self::name(cfg), Self::inline(cfg))
    }
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
    /// Owner-facing explanation naming the rule code, never containing projection values.
    pub message: String,
    /// Sanitized error safe to send back to the model for repair: carries only
    /// the error class and offending identifiers from the model's own SQL.
    pub model_error: String,
    /// Local log detail naming the rule/class, never containing projection values.
    pub detail: String,
}

impl SandboxRejection {
    fn new(
        rule: SandboxRule,
        message: impl Into<String>,
        model_error: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            rule,
            message: message.into(),
            model_error: model_error.into(),
            detail: detail.into(),
        }
    }

    pub fn rule_name(&self) -> &'static str {
        self.rule.rule_name()
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
    "fold",
    "format",
    "glob",
    "hex",
    "ifnull",
    "iif",
    "instr",
    "length",
    "like",
    "likelihood",
    "likely",
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
    "unistr",
    "unistr_quote",
    "unlikely",
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
    "json_quote",
    "json_tree",
    "json_type",
    "json_valid",
];

fn is_allowed_function(name: &str) -> bool {
    ALLOWED_FUNCTIONS
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(name))
}

/// Skip leading ASCII whitespace and SQL comments (`-- ...` and `/* ... */`)
/// and verify the statement starts with `SELECT` or `WITH`, contains no bound
/// parameter placeholders, and contains at most one statement.
fn static_check(sql: &str, max_bytes: usize) -> Result<Vec<String>, SandboxRejection> {
    if sql.contains('\0') {
        return Err(SandboxRejection::new(
            SandboxRule::NulByte,
            "The SQL query cannot contain NUL bytes (rule `nul_byte`).",
            "nul_byte: query contains a NUL byte",
            "nul_byte",
        ));
    }
    if sql.len() > max_bytes {
        return Err(SandboxRejection::new(
            SandboxRule::SqlTooLong,
            format!("The SQL query exceeds the {max_bytes}-byte limit (rule `sql_too_long`)."),
            format!("sql_too_long: query exceeds {max_bytes}-byte limit"),
            "sql_too_long",
        ));
    }
    let trimmed = skip_leading_ws_and_comments(sql);
    if trimmed.is_empty() {
        return Err(SandboxRejection::new(
            SandboxRule::EmptySql,
            "The SQL query is empty (rule `empty_sql`).",
            "empty_sql: query is empty",
            "empty_sql",
        ));
    }
    if !starts_with_keyword(trimmed, "SELECT") && !starts_with_keyword(trimmed, "WITH") {
        return Err(SandboxRejection::new(
            SandboxRule::NotSelect,
            "Only SELECT or WITH queries are allowed (rule `not_select`).",
            "not_select: only SELECT or WITH queries are allowed",
            "not_select",
        ));
    }
    scan_static_safety(trimmed)?;
    Ok(extract_cte_names(trimmed))
}

/// Scan SQL outside literals, quoted identifiers, and comments for multiple
/// statements and bound parameter placeholders (`?`, `:x`, `@x`, `$x`).
fn scan_static_safety(sql: &str) -> Result<(), SandboxRejection> {
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut saw_semi = false;
    while i < bytes.len() {
        if saw_semi {
            let rest = skip_leading_ws_and_comments(&sql[i..]);
            if !rest.is_empty() {
                return Err(SandboxRejection::new(
                    SandboxRule::MultipleStatements,
                    "Only a single SQL statement is allowed (rule `multiple_statements`).",
                    "multiple_statements: only a single SQL statement is allowed",
                    "multiple_statements",
                ));
            }
            return Ok(());
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
            b'?' | b':' | b'@' | b'$' => {
                return Err(SandboxRejection::new(
                    SandboxRule::BoundParameters,
                    "Bound parameter placeholders are not allowed in Ask SQL (rule `bound_parameters`).",
                    "bound_parameters: bound parameter placeholders are not allowed",
                    "bound_parameters",
                ));
            }
            b';' => {
                saw_semi = true;
                i += 1;
            }
            _ => {
                i += sql[i..].chars().next().map_or(1, char::len_utf8);
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
enum WithPhase {
    ExpectNameOrRecursive,
    ExpectName,
    AfterName,
    InColList,
    ExpectAs,
    ExpectBodyOpen,
    InBody(usize),
    AfterBody,
}

#[derive(Debug)]
struct WithFrame {
    depth: usize,
    phase: WithPhase,
    pending_name: Option<String>,
}

/// Extract CTE names declared in `WITH [RECURSIVE] <name> [(cols)] AS (...)`
/// clauses so the authorizer can distinguish CTE references from base tables.
fn extract_cte_names(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut names = Vec::new();
    let mut stack: Vec<WithFrame> = Vec::new();
    let mut paren_depth: usize = 0;
    let mut can_start_with = true;

    while i < bytes.len() {
        let rest = &sql[i..];
        let trimmed = skip_leading_ws_and_comments(rest);
        i += rest.len() - trimmed.len();
        if i >= bytes.len() {
            break;
        }
        // An unclosed block comment stops scanning cleanly on char boundaries.
        if bytes.get(i) == Some(&b'/') && bytes.get(i + 1) == Some(&b'*') {
            break;
        }
        match bytes[i] {
            b'\'' | b'"' | b'`' => {
                let quote = bytes[i];
                i += 1;
                let mut unescaped: Vec<u8> = Vec::new();
                while i < bytes.len() {
                    if bytes[i] == quote {
                        if bytes.get(i + 1) == Some(&quote) {
                            unescaped.push(quote);
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    unescaped.push(bytes[i]);
                    i += 1;
                }
                if quote != b'\''
                    && let Some(top) = stack.last_mut()
                    && top.depth == paren_depth
                    && matches!(
                        top.phase,
                        WithPhase::ExpectNameOrRecursive | WithPhase::ExpectName
                    )
                {
                    top.pending_name = Some(String::from_utf8_lossy(&unescaped).into_owned());
                    top.phase = WithPhase::AfterName;
                }
                can_start_with = false;
            }
            b'[' => {
                let start = i + 1;
                i += 1;
                while i < bytes.len() && bytes[i] != b']' {
                    i += 1;
                }
                let ident = String::from_utf8_lossy(&bytes[start..i]).into_owned();
                if i < bytes.len() {
                    i += 1;
                }
                if let Some(top) = stack.last_mut()
                    && top.depth == paren_depth
                    && matches!(
                        top.phase,
                        WithPhase::ExpectNameOrRecursive | WithPhase::ExpectName
                    )
                {
                    top.pending_name = Some(ident);
                    top.phase = WithPhase::AfterName;
                }
                can_start_with = false;
            }
            b'(' => {
                paren_depth += 1;
                i += 1;
                let parent_depth = paren_depth - 1;
                let mut handled = false;
                if let Some(top) = stack.last_mut()
                    && top.depth == parent_depth
                {
                    match top.phase {
                        WithPhase::AfterName => {
                            top.phase = WithPhase::InColList;
                            handled = true;
                        }
                        WithPhase::ExpectBodyOpen => {
                            if let Some(name) = top.pending_name.take() {
                                names.push(name);
                            }
                            top.phase = WithPhase::InBody(paren_depth);
                            handled = true;
                        }
                        _ => {}
                    }
                }
                if !handled {
                    can_start_with = true;
                } else {
                    can_start_with =
                        matches!(stack.last().map(|f| &f.phase), Some(WithPhase::InBody(_)));
                }
            }
            b')' => {
                paren_depth = paren_depth.saturating_sub(1);
                i += 1;
                while let Some(top) = stack.last() {
                    if top.depth > paren_depth {
                        stack.pop();
                    } else {
                        break;
                    }
                }
                if let Some(top) = stack.last_mut()
                    && top.depth == paren_depth
                {
                    match top.phase {
                        WithPhase::InColList => top.phase = WithPhase::ExpectAs,
                        WithPhase::InBody(body_depth) if paren_depth < body_depth => {
                            top.phase = WithPhase::AfterBody;
                        }
                        _ => {}
                    }
                }
                can_start_with = false;
            }
            b',' => {
                i += 1;
                if let Some(top) = stack.last_mut()
                    && top.depth == paren_depth
                    && matches!(top.phase, WithPhase::AfterBody)
                {
                    top.phase = WithPhase::ExpectName;
                }
                can_start_with = false;
            }
            b if b.is_ascii_alphanumeric() || b == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let word = &sql[start..i];
                let handled = if let Some(top) = stack.last_mut()
                    && top.depth == paren_depth
                {
                    match top.phase {
                        WithPhase::ExpectNameOrRecursive => {
                            if word.eq_ignore_ascii_case("RECURSIVE") {
                                top.phase = WithPhase::ExpectName;
                            } else {
                                top.pending_name = Some(word.to_owned());
                                top.phase = WithPhase::AfterName;
                            }
                            true
                        }
                        WithPhase::ExpectName => {
                            top.pending_name = Some(word.to_owned());
                            top.phase = WithPhase::AfterName;
                            true
                        }
                        WithPhase::AfterName | WithPhase::ExpectAs => {
                            if word.eq_ignore_ascii_case("AS") {
                                top.phase = WithPhase::ExpectBodyOpen;
                            } else {
                                stack.pop();
                            }
                            true
                        }
                        WithPhase::ExpectBodyOpen => {
                            if word.eq_ignore_ascii_case("NOT")
                                || word.eq_ignore_ascii_case("MATERIALIZED")
                            {
                                // Stay in ExpectBodyOpen for `AS [NOT] MATERIALIZED (...)`.
                            } else {
                                stack.pop();
                            }
                            true
                        }
                        WithPhase::AfterBody => {
                            if matches!(
                                word.to_ascii_uppercase().as_str(),
                                "SELECT" | "VALUES" | "INSERT" | "UPDATE" | "DELETE"
                            ) {
                                stack.pop();
                            }
                            true
                        }
                        WithPhase::InColList | WithPhase::InBody(_) => false,
                    }
                } else {
                    false
                };
                if !handled && can_start_with && word.eq_ignore_ascii_case("WITH") {
                    stack.push(WithFrame {
                        depth: paren_depth,
                        phase: WithPhase::ExpectNameOrRecursive,
                        pending_name: None,
                    });
                    can_start_with = false;
                    continue;
                }
                can_start_with = matches!(
                    word.to_ascii_uppercase().as_str(),
                    "UNION" | "ALL" | "INTERSECT" | "EXCEPT"
                );
            }
            _ => {
                i += sql[i..].chars().next().map_or(1, char::len_utf8);
                can_start_with = false;
            }
        }
    }
    names
}

fn skip_leading_ws_and_comments(mut s: &str) -> &str {
    loop {
        s = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
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
         PRAGMA temp.max_page_count = 1024;\
         PRAGMA cache_size = -2000;\
         PRAGMA cell_size_check = ON;",
    )?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DML, false)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DDL, false)?;
    // Explicitly disable SQL/C extension loading at the SQLite db_config level.
    unsafe {
        let rc = libsqlite3_sys::sqlite3_db_config(
            conn.handle(),
            libsqlite3_sys::SQLITE_DBCONFIG_ENABLE_LOAD_EXTENSION,
            0 as std::ffi::c_int,
            std::ptr::null_mut::<std::ffi::c_int>(),
        );
        if rc != libsqlite3_sys::SQLITE_OK {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rc),
                Some("failed to disable extension loading".into()),
            ));
        }
    }

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
    conn.set_limit(Limit::SQLITE_LIMIT_WORKER_THREADS, 0)?;
    conn.set_limit(Limit::SQLITE_LIMIT_VDBE_OP, 25_000)?;
    projection::register_text_functions(conn)?;
    Ok(())
}

type DeniedState = Arc<Mutex<Option<(SandboxRule, String)>>>;

const FORBIDDEN_EPONYMOUS_VTABLES: &[&str] = &[
    "dbstat",
    "bytecode",
    "tables_used",
    "stmt",
    "completion",
    "fsdir",
    "sqlite_dbpage",
    "sqlite_memstat",
];

fn authorize(ctx: AuthContext<'_>, cte_names: &[String], denied: &DeniedState) -> Authorization {
    let record = |rule: SandboxRule, detail: String| {
        let mut slot = denied.lock().unwrap_or_else(|poison| poison.into_inner());
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
            if let Some(db) = ctx.database_name
                && !db.eq_ignore_ascii_case("main")
            {
                return record(
                    SandboxRule::UnauthorizedTable,
                    format!("table {db}.{table_name} is not in the Ask projection"),
                );
            }
            if (table_name.eq_ignore_ascii_case("json_each")
                || table_name.eq_ignore_ascii_case("json_tree"))
                && ctx
                    .database_name
                    .is_none_or(|db| db.eq_ignore_ascii_case("main"))
            {
                return Authorization::Allow;
            }
            let lower = table_name.to_ascii_lowercase();
            if FORBIDDEN_EPONYMOUS_VTABLES.contains(&lower.as_str()) {
                return record(
                    SandboxRule::UnauthorizedTable,
                    format!("table {table_name} is not in the Ask projection"),
                );
            }
            if ctx.database_name.is_none()
                && !lower.starts_with("sqlite_")
                && !lower.starts_with("pragma_")
                && !table_name.eq_ignore_ascii_case("_projection_meta")
                && cte_names
                    .iter()
                    .any(|cte| cte.eq_ignore_ascii_case(table_name))
            {
                return Authorization::Allow;
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

const INTERRUPT_NONE: u8 = 0;
const INTERRUPT_TIMEOUT: u8 = 1;
const INTERRUPT_INSTRUCTION_LIMIT: u8 = 2;
const INTERRUPT_CANCELLED: u8 = 3;

#[derive(Clone, Copy)]
enum QueryPhase {
    Prepare,
    Runtime,
}

fn ident_from_sql(raw: &str, sql: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches("()");
    if trimmed.is_empty() || trimmed.len() > 64 || trimmed.chars().any(char::is_control) {
        return None;
    }
    if sql.contains(trimmed) {
        return Some(trimmed.to_owned());
    }
    if let Some((_, suffix)) = trimmed.rsplit_once('.')
        && !suffix.is_empty()
        && sql.contains(suffix)
    {
        return Some(trimmed.to_owned());
    }
    None
}

fn sanitize_prepare_error(
    err: rusqlite::ffi::Error,
    msg: Option<&str>,
    sql: &str,
) -> SandboxRejection {
    let text = msg.unwrap_or("");
    if let Some(table) = text.strip_prefix("no such table: ") {
        let safe_table = ident_from_sql(table, sql).unwrap_or_else(|| "unknown".into());
        return SandboxRejection::new(
            SandboxRule::UnauthorizedTable,
            format!(
                "Query rejected by rule `unauthorized_table`: table {safe_table} is not in the Ask projection."
            ),
            format!("unauthorized_table: table {safe_table} is not in the Ask projection"),
            format!("no_such_table:{safe_table}"),
        );
    }
    if let Some(col) = text.strip_prefix("no such column: ") {
        let safe_col = ident_from_sql(col, sql).unwrap_or_else(|| "unknown".into());
        return SandboxRejection::new(
            SandboxRule::SqlError,
            format!("The SQL query could not run: no such column `{safe_col}` (rule `sql_error`)."),
            format!("sql_error (no_such_column): {safe_col}"),
            format!("no_such_column:{safe_col}"),
        );
    }
    if let Some(col) = text.strip_prefix("ambiguous column name: ") {
        let safe_col = ident_from_sql(col, sql).unwrap_or_else(|| "unknown".into());
        return SandboxRejection::new(
            SandboxRule::SqlError,
            format!(
                "The SQL query could not run: ambiguous column `{safe_col}` (rule `sql_error`)."
            ),
            format!("sql_error (ambiguous_column): {safe_col}"),
            format!("ambiguous_column:{safe_col}"),
        );
    }
    if let Some(fn_name) = text.strip_prefix("no such function: ") {
        let safe_fn = ident_from_sql(fn_name, sql).unwrap_or_else(|| "unknown".into());
        return SandboxRejection::new(
            SandboxRule::SqlError,
            format!(
                "The SQL query could not run: no such function `{safe_fn}` (rule `sql_error`)."
            ),
            format!("sql_error (no_such_function): {safe_fn}"),
            format!("no_such_function:{safe_fn}"),
        );
    }
    if let Some(rest) = text.strip_prefix("wrong number of arguments to function ") {
        let safe_fn = ident_from_sql(rest, sql).unwrap_or_else(|| "unknown".into());
        return SandboxRejection::new(
            SandboxRule::SqlError,
            format!(
                "The SQL query could not run: wrong number of arguments to `{safe_fn}` (rule `sql_error`)."
            ),
            format!("sql_error (wrong_argument_count): {safe_fn}"),
            format!("wrong_argument_count:{safe_fn}"),
        );
    }
    if let Some(rest) = text
        .strip_prefix("misuse of aggregate: ")
        .or_else(|| text.strip_prefix("misuse of window function: "))
        .or_else(|| text.strip_prefix("misuse of aliased aggregate "))
    {
        let safe_fn = ident_from_sql(rest, sql).unwrap_or_else(|| "unknown".into());
        return SandboxRejection::new(
            SandboxRule::SqlError,
            format!("The SQL query could not run: invalid use of `{safe_fn}` (rule `sql_error`)."),
            format!("sql_error (aggregate_misuse): {safe_fn}"),
            format!("aggregate_misuse:{safe_fn}"),
        );
    }
    if let Some(rest) = text
        .strip_prefix("near \"")
        .and_then(|s| s.strip_suffix("\": syntax error"))
    {
        let model_err = match ident_from_sql(rest, sql) {
            Some(tok) => format!("sql_error (syntax_error): near \"{tok}\""),
            None => "sql_error (syntax_error)".to_owned(),
        };
        return SandboxRejection::new(
            SandboxRule::SqlError,
            "The SQL query has a syntax error (rule `sql_error`).",
            model_err,
            "syntax_error",
        );
    }
    if text == "incomplete input" {
        return SandboxRejection::new(
            SandboxRule::SqlError,
            "The SQL query is incomplete (rule `sql_error`).",
            "sql_error (incomplete_input)",
            "incomplete_input",
        );
    }
    if text.contains("Expression tree is too large") || text.contains("parser stack overflow") {
        return SandboxRejection::new(
            SandboxRule::SqlError,
            "The SQL query expression nesting exceeds the sandbox limit (rule `sql_error`).",
            "sql_error (expression_depth_limit)",
            "expression_depth_limit",
        );
    }
    if text.contains("too many terms in compound SELECT") {
        return SandboxRejection::new(
            SandboxRule::SqlError,
            "The SQL query has too many compound SELECT terms (rule `sql_error`).",
            "sql_error (compound_select_limit)",
            "compound_select_limit",
        );
    }
    if text.contains("do not have the same number of result columns") {
        return SandboxRejection::new(
            SandboxRule::SqlError,
            "Compound SELECT branches must return the same number of columns (rule `sql_error`).",
            "sql_error (compound_column_mismatch)",
            "compound_column_mismatch",
        );
    }
    if text.contains("too many arguments on function") {
        return SandboxRejection::new(
            SandboxRule::SqlError,
            "A function call in the SQL query exceeds the argument limit (rule `sql_error`).",
            "sql_error (function_arg_limit)",
            "function_arg_limit",
        );
    }
    if text.contains("too many columns") {
        return SandboxRejection::new(
            SandboxRule::SqlError,
            "The SQL query selects too many columns (rule `sql_error`).",
            "sql_error (column_limit)",
            "column_limit",
        );
    }
    if text.contains("term out of range") {
        return SandboxRejection::new(
            SandboxRule::SqlError,
            "An ORDER BY or GROUP BY term is out of range (rule `sql_error`).",
            "sql_error (term_out_of_range)",
            "term_out_of_range",
        );
    }
    if text.contains("too many SQL variables") {
        return SandboxRejection::new(
            SandboxRule::BoundParameters,
            "Bound parameter placeholders are not allowed in Ask SQL (rule `bound_parameters`).",
            "bound_parameters: bound parameter placeholders are not allowed",
            "bound_parameters",
        );
    }
    if text.contains("LIKE or GLOB pattern") {
        return SandboxRejection::new(
            SandboxRule::SqlError,
            "A LIKE or GLOB pattern exceeds the sandbox limit (rule `sql_error`).",
            "sql_error (pattern_limit)",
            "pattern_limit",
        );
    }
    if text.contains("sub-select returns") || text.contains("row value misused") {
        return SandboxRejection::new(
            SandboxRule::SqlError,
            "A scalar subquery returned an unexpected number of columns (rule `sql_error`).",
            "sql_error (subquery_column_count): scalar subquery must return a single column",
            "subquery_column_count",
        );
    }
    if text.contains("circularly defined") {
        return SandboxRejection::new(
            SandboxRule::SqlError,
            "A CTE in the SQL query is circularly defined (rule `sql_error`).",
            "sql_error (circular_cte): CTE is circularly defined",
            "circular_cte",
        );
    }
    if text.contains("recursive aggregate") {
        return SandboxRejection::new(
            SandboxRule::SqlError,
            "Aggregate functions are not allowed inside recursive CTE terms (rule `sql_error`).",
            "sql_error (recursive_aggregate): aggregate functions are not allowed in recursive CTE terms",
            "recursive_aggregate",
        );
    }
    let class = format!("prepare_error:{:?}", err.code);
    SandboxRejection::new(
        SandboxRule::SqlError,
        "The SQL query could not be prepared (rule `sql_error`).",
        format!("sql_error ({class})"),
        class,
    )
}

fn sanitize_runtime_error(err: rusqlite::ffi::Error, msg: Option<&str>) -> SandboxRejection {
    // Runtime errors occur while scanning projection rows: never copy dynamic
    // SQLite message substrings that could originate from a row value.
    let text = msg.unwrap_or("");
    let (class, owner) = if text == "malformed JSON" {
        (
            "malformed_json",
            "The SQL query encountered malformed JSON input (rule `sql_error`).",
        )
    } else if text.contains("JSON") {
        (
            "json_error",
            "A JSON function failed while evaluating the SQL query (rule `sql_error`).",
        )
    } else if text.contains("LIKE or GLOB pattern") {
        (
            "pattern_limit",
            "A LIKE or GLOB pattern exceeds the sandbox limit (rule `sql_error`).",
        )
    } else if text.contains("ESCAPE expression") {
        (
            "invalid_like_escape",
            "Invalid LIKE ESCAPE expression in SQL query (rule `sql_error`).",
        )
    } else if text.contains("nth_value") || text.contains("ntile") || text.contains("frame") {
        (
            "window_argument_error",
            "Invalid window function argument in SQL query (rule `sql_error`).",
        )
    } else if err.code == rusqlite::ErrorCode::TypeMismatch || text.contains("datatype mismatch") {
        (
            "datatype_mismatch",
            "The SQL query encountered a datatype mismatch (rule `sql_error`).",
        )
    } else {
        (
            "runtime_error",
            "The SQL query failed during execution (rule `sql_error`).",
        )
    };
    SandboxRejection::new(
        SandboxRule::SqlError,
        owner,
        format!("sql_error ({class})"),
        class,
    )
}

fn map_sqlite_error(
    error: rusqlite::Error,
    phase: QueryPhase,
    sql: &str,
    denied: &DeniedState,
    interrupt_reason: &AtomicU8,
    cancel: Option<&SandboxCancel>,
) -> SandboxRejection {
    if let Some((rule, detail)) = denied
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take()
    {
        let name = rule.rule_name();
        let message = format!("Query rejected by rule `{name}`: {detail}.");
        let model_error = format!("{name}: {detail}");
        return SandboxRejection::new(rule, message, model_error, detail);
    }
    match error {
        rusqlite::Error::MultipleStatement => SandboxRejection::new(
            SandboxRule::MultipleStatements,
            "Only a single SQL statement is allowed (rule `multiple_statements`).",
            "multiple_statements: only a single SQL statement is allowed",
            "multiple_statements",
        ),
        rusqlite::Error::SqliteFailure(err, msg) => {
            if err.code == rusqlite::ErrorCode::OperationInterrupted {
                if cancel.is_some_and(SandboxCancel::is_cancelled)
                    || interrupt_reason.load(Ordering::SeqCst) == INTERRUPT_CANCELLED
                {
                    return SandboxRejection::new(
                        SandboxRule::Cancelled,
                        "The SQL query was cancelled (rule `cancelled`).",
                        "cancelled: query was cancelled",
                        "cancelled",
                    );
                }
                if interrupt_reason.load(Ordering::SeqCst) == INTERRUPT_INSTRUCTION_LIMIT {
                    return SandboxRejection::new(
                        SandboxRule::InstructionLimit,
                        "The SQL query exceeded the instruction budget and was stopped (rule `instruction_limit`).",
                        "instruction_limit: query exceeded instruction budget",
                        "instruction_limit",
                    );
                }
                return SandboxRejection::new(
                    SandboxRule::Timeout,
                    "The SQL query exceeded the time limit and was stopped (rule `timeout`).",
                    "timeout: query exceeded wall-clock time limit",
                    "timeout",
                );
            }
            if err.code == rusqlite::ErrorCode::AuthorizationForStatementDenied {
                return SandboxRejection::new(
                    SandboxRule::UnauthorizedAction,
                    "The SQL query was denied by the sandbox authorizer (rule `unauthorized_action`).",
                    "unauthorized_action: statement denied by sandbox authorizer",
                    "unauthorized_action",
                );
            }
            if err.code == rusqlite::ErrorCode::TooBig {
                return SandboxRejection::new(
                    SandboxRule::OutputTooLarge,
                    "The SQL query produced a value exceeding the size limit (rule `output_too_large`).",
                    "output_too_large: query result exceeded size limit",
                    "output_too_large",
                );
            }
            match phase {
                QueryPhase::Prepare => sanitize_prepare_error(err, msg.as_deref(), sql),
                QueryPhase::Runtime => sanitize_runtime_error(err, msg.as_deref()),
            }
        }
        other => {
            let class = format!("{:?}", other.sqlite_error_code());
            SandboxRejection::new(
                SandboxRule::SqlError,
                "The SQL query could not run (rule `sql_error`).",
                format!("sql_error ({class})"),
                class,
            )
        }
    }
}

/// Run a single read-only SQL query against the Ask projection file inside the
/// sandbox. This is the only entry point for Ask SQL execution.
pub fn execute(
    projection_path: &Path,
    sql: &str,
    limits: SandboxLimits,
) -> std::result::Result<SandboxOutput, SandboxRejection> {
    execute_with_cancel(projection_path, sql, limits, None)
}

/// Run a single read-only SQL query against the Ask projection file inside the
/// sandbox with an optional cancellation handle that interrupts the SQLite
/// connection if triggered.
pub fn execute_with_cancel(
    projection_path: &Path,
    sql: &str,
    limits: SandboxLimits,
    cancel: Option<&SandboxCancel>,
) -> std::result::Result<SandboxOutput, SandboxRejection> {
    let result = execute_inner(projection_path, sql, limits, cancel);
    if let Err(ref rejection) = result {
        eprintln!(
            "Ask SQL rejected ({}): {}",
            rejection.rule_name(),
            rejection.detail
        );
    }
    result
}

#[expect(
    clippy::disallowed_methods,
    reason = "canonical read-only Ask SQL sandbox boundary"
)]
fn execute_inner(
    projection_path: &Path,
    sql: &str,
    limits: SandboxLimits,
    cancel: Option<&SandboxCancel>,
) -> std::result::Result<SandboxOutput, SandboxRejection> {
    if cancel.is_some_and(SandboxCancel::is_cancelled) {
        return Err(SandboxRejection::new(
            SandboxRule::Cancelled,
            "The SQL query was cancelled (rule `cancelled`).",
            "cancelled: query was cancelled",
            "cancelled",
        ));
    }

    let cte_names = static_check(sql, limits.max_sql_bytes)?;

    if !projection_path.is_file() {
        return Err(SandboxRejection::new(
            SandboxRule::ProjectionUnavailable,
            "The Ask projection is not built yet (rule `projection_unavailable`).",
            "projection_unavailable: Ask projection is not built yet",
            "projection_missing",
        ));
    }

    let conn = Connection::open_with_flags(
        projection_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| {
        SandboxRejection::new(
            SandboxRule::ProjectionUnavailable,
            "Could not open the Ask projection (rule `projection_unavailable`).",
            "projection_unavailable: could not open Ask projection",
            "projection_open_failed",
        )
    })?;

    let _interrupt_guard =
        cancel.map(|token| InterruptRegistration::register(token, conn.get_interrupt_handle()));

    configure_connection(&conn, limits).map_err(|_| {
        SandboxRejection::new(
            SandboxRule::SqlError,
            "Failed to configure the SQL sandbox (rule `sql_error`).",
            "sql_error (sandbox_configuration)",
            "sandbox_configuration_failed",
        )
    })?;

    let denied: DeniedState = Arc::new(Mutex::new(None));
    let auth_denied = Arc::clone(&denied);
    conn.authorizer(Some(move |ctx: AuthContext<'_>| {
        authorize(ctx, &cte_names, &auth_denied)
    }))
    .map_err(|err| {
        map_sqlite_error(
            err,
            QueryPhase::Prepare,
            sql,
            &denied,
            &AtomicU8::new(INTERRUPT_NONE),
            cancel,
        )
    })?;

    let deadline = Instant::now() + limits.timeout;
    let interrupt_reason = Arc::new(AtomicU8::new(INTERRUPT_NONE));
    let reason_ref = Arc::clone(&interrupt_reason);
    let instructions = AtomicU64::new(0);
    let max_instructions = limits.max_instructions;
    let cancel_flag = cancel.map(|token| Arc::clone(&token.cancelled));
    conn.progress_handler(
        PROGRESS_STEP_INTERVAL,
        Some(move || {
            if cancel_flag
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::SeqCst))
            {
                reason_ref.store(INTERRUPT_CANCELLED, Ordering::SeqCst);
                return true;
            }
            if Instant::now() >= deadline {
                reason_ref.store(INTERRUPT_TIMEOUT, Ordering::SeqCst);
                return true;
            }
            let prev = instructions.fetch_add(PROGRESS_STEP_INTERVAL as u64, Ordering::Relaxed);
            if prev.saturating_add(PROGRESS_STEP_INTERVAL as u64) > max_instructions {
                reason_ref.store(INTERRUPT_INSTRUCTION_LIMIT, Ordering::SeqCst);
                return true;
            }
            false
        }),
    )
    .map_err(|_| {
        SandboxRejection::new(
            SandboxRule::SqlError,
            "Failed to configure the SQL timeout handler (rule `sql_error`).",
            "sql_error (progress_handler_configuration)",
            "progress_handler_failed",
        )
    })?;

    let mut stmt = conn.prepare(sql).map_err(|err| {
        map_sqlite_error(
            err,
            QueryPhase::Prepare,
            sql,
            &denied,
            &interrupt_reason,
            cancel,
        )
    })?;

    if !stmt.readonly() {
        return Err(SandboxRejection::new(
            SandboxRule::NonReadOnly,
            "Only read-only statements are allowed (rule `non_read_only`).",
            "non_read_only: only read-only statements are allowed",
            "non_read_only",
        ));
    }

    let columns: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
    let col_count = columns.len();

    let mut query_rows = stmt.query([]).map_err(|err| {
        map_sqlite_error(
            err,
            QueryPhase::Runtime,
            sql,
            &denied,
            &interrupt_reason,
            cancel,
        )
    })?;

    let mut rows = Vec::new();
    let mut used_bytes: usize = 0;
    let mut truncated = false;
    while let Some(row) = query_rows.next().map_err(|err| {
        map_sqlite_error(
            err,
            QueryPhase::Runtime,
            sql,
            &denied,
            &interrupt_reason,
            cancel,
        )
    })? {
        if rows.len() >= limits.max_rows {
            truncated = true;
            break;
        }
        let mut values = Vec::with_capacity(col_count);
        let mut row_bytes: usize = 0;
        for index in 0..col_count {
            let cell = row.get_ref(index).map_err(|err| {
                map_sqlite_error(
                    err,
                    QueryPhase::Runtime,
                    sql,
                    &denied,
                    &interrupt_reason,
                    cancel,
                )
            })?;
            let value = match cell {
                ValueRef::Null => SqlValue::Null,
                ValueRef::Integer(i) => {
                    row_bytes = row_bytes.saturating_add(8);
                    SqlValue::Integer(i)
                }
                ValueRef::Real(f) => {
                    row_bytes = row_bytes.saturating_add(8);
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
                            "Query returned invalid UTF-8 text (rule `sql_error`).",
                            "sql_error (invalid_utf8)",
                            "invalid_utf8",
                        )
                    })?;
                    row_bytes = row_bytes.saturating_add(text.len());
                    SqlValue::Text(text.to_owned())
                }
                ValueRef::Blob(_) => {
                    return Err(SandboxRejection::new(
                        SandboxRule::BlobNotAllowed,
                        "Binary BLOB values are not allowed in Ask results (rule `blob_not_allowed`).",
                        "blob_not_allowed: binary BLOB values are not allowed in results",
                        "blob_not_allowed",
                    ));
                }
            };
            values.push(value);
        }
        if used_bytes.saturating_add(row_bytes) > limits.max_bytes {
            if rows.is_empty() {
                return Err(SandboxRejection::new(
                    SandboxRule::OutputTooLarge,
                    format!(
                        "Query row exceeded the {}-byte output cap (rule `output_too_large`).",
                        limits.max_bytes
                    ),
                    "output_too_large: query row exceeded output byte cap",
                    "output_too_large",
                ));
            }
            truncated = true;
            break;
        }
        used_bytes = used_bytes.saturating_add(row_bytes);
        rows.push(values);
    }

    Ok(SandboxOutput {
        columns,
        rows,
        truncated,
    })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::ask::projection;
    use rusqlite::Connection;
    use tempfile::NamedTempFile;

    #[test]
    fn every_rule_code_matches_the_serde_wire_name() {
        for rule in [
            SandboxRule::EmptySql,
            SandboxRule::SqlTooLong,
            SandboxRule::NulByte,
            SandboxRule::NotSelect,
            SandboxRule::MultipleStatements,
            SandboxRule::BoundParameters,
            SandboxRule::NonReadOnly,
            SandboxRule::UnauthorizedAction,
            SandboxRule::UnauthorizedTable,
            SandboxRule::UnauthorizedColumn,
            SandboxRule::Timeout,
            SandboxRule::InstructionLimit,
            SandboxRule::Cancelled,
            SandboxRule::BlobNotAllowed,
            SandboxRule::OutputTooLarge,
            SandboxRule::SqlError,
            SandboxRule::ProjectionUnavailable,
        ] {
            assert_eq!(
                serde_json::to_value(rule).unwrap(),
                serde_json::Value::String(rule.rule_name().into()),
                "{rule:?}"
            );
        }
        assert!(!SandboxRule::Timeout.is_repairable());
        assert!(!SandboxRule::Cancelled.is_repairable());
        assert!(!SandboxRule::ProjectionUnavailable.is_repairable());
        assert!(SandboxRule::SqlError.is_repairable());
        assert!(SandboxRule::UnauthorizedTable.is_repairable());
    }

    fn fixture_projection() -> NamedTempFile {
        let file = NamedTempFile::new().unwrap();
        let conn = Connection::open(file.path()).unwrap();
        conn.execute_batch(&projection::ddl()).unwrap();
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
            "INSERT INTO tracks(id, title, album_id, duration_ms, edition_key) VALUES ('t1', 'Jóga', 'al1', 305000, 'joga:a1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO track_artists(track_id, artist_id, ordinal) VALUES ('t1', 'a1', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO album_artists(album_id, artist_id, ordinal) VALUES ('al1', 'a1', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO listens(id, track_id, played_at_ms, played_at_local, local_date, week_date, local_year, local_month, weekday, week_start, local_hour, listened_ms, duration_kind, counted, needs_review, sources) VALUES ('l1', 't1', 1704880800000, '2024-01-10 11:00:00', '2024-01-10', '2024-01-08', 2024, 1, 3, 0, 11, 305000, 'actual', 1, 0, '[\"spotify_api\"]')",
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

    #[test]
    fn recursive_cte_selecting_columns_succeeds_while_eponymous_vtables_are_denied() {
        let proj = fixture_projection();
        let out = execute(
            proj.path(),
            "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 3) SELECT x FROM cnt",
            SandboxLimits::default(),
        )
        .expect("legitimate bounded recursive CTE selecting column should succeed");
        assert_eq!(
            out.rows,
            vec![
                vec![SqlValue::Integer(1)],
                vec![SqlValue::Integer(2)],
                vec![SqlValue::Integer(3)],
            ]
        );

        for sql in [
            "SELECT * FROM dbstat",
            "SELECT COUNT(*) FROM dbstat",
            "SELECT 1 AS with, name FROM dbstat",
            "SELECT (WITH dbstat AS (SELECT 1) SELECT * FROM dbstat), * FROM dbstat",
            "WITH x AS (SELECT * FROM dbstat) SELECT * FROM x",
            "SELECT * FROM bytecode('SELECT 1')",
            "SELECT * FROM tables_used('SELECT 1 FROM listens')",
            "SELECT * FROM completion('SEL')",
        ] {
            let err = execute(proj.path(), sql, SandboxLimits::default())
                .expect_err(&format!("eponymous vtable must be rejected: {sql}"));
            assert_eq!(err.rule, SandboxRule::UnauthorizedTable, "{sql}: {err:?}");
        }
    }

    #[test]
    fn non_ascii_utf8_sql_tokens_and_comments_never_panic() {
        let proj = fixture_projection();

        // Multi-byte UTF-8 outside literals/comments (em-dash, accented aliases, CJK) must not panic.
        for sql in [
            "SELECT 1 AS café",
            "SELECT COUNT(*) AS \"Björk — 音楽\" FROM listens",
            "WITH \"café_counts\" AS (SELECT COUNT(*) AS c FROM listens) SELECT c FROM \"café_counts\"",
        ] {
            let out = execute(proj.path(), sql, SandboxLimits::default())
                .unwrap_or_else(|e| panic!("valid UTF-8 query should not fail ({sql}): {e:?}"));
            assert!(!out.columns.is_empty());
        }

        // Unclosed strings or invalid UTF-8 tokens after syntax characters must reject cleanly without panicking.
        for sql in ["SELECT 1 — 2", "SELECT 'café", "SELECT 1 /* — */ FROM"] {
            let err = execute(proj.path(), sql, SandboxLimits::default()).expect_err(&format!(
                "invalid syntax with UTF-8 must reject cleanly: {sql}"
            ));
            assert_eq!(err.rule, SandboxRule::SqlError, "{sql}: {err:?}");
        }
    }

    #[test]
    fn bound_parameters_and_unicode_tricks_are_rejected_while_punctuation_in_strings_works() {
        let proj = fixture_projection();

        for sql in [
            "SELECT ?",
            "SELECT ?1",
            "SELECT :name",
            "SELECT @name",
            "SELECT $name",
            "SELECT * FROM listens WHERE id = ?",
            "SELECT * FROM artists WHERE name = :artist",
        ] {
            let err = execute(proj.path(), sql, SandboxLimits::default())
                .expect_err(&format!("bound parameter must be rejected: {sql}"));
            assert_eq!(err.rule, SandboxRule::BoundParameters, "{sql}: {err:?}");
        }

        // String literals and quoted identifiers containing parameter sigils must work.
        let out = execute(
            proj.path(),
            "SELECT 'who? user@host :1 $100' AS \"col?\"",
            SandboxLimits::default(),
        )
        .expect("punctuation inside string literals and identifiers should be allowed");
        assert_eq!(out.columns, vec!["col?"]);
        assert_eq!(
            out.rows,
            vec![vec![SqlValue::Text("who? user@host :1 $100".into())]]
        );

        // Unicode whitespace, BOM, zero-width, and fullwidth tricks must be rejected at static check.
        for sql in [
            "\u{FEFF}SELECT 1",
            "\u{00A0}SELECT 1",
            "\u{200B}SELECT 1",
            "ＳＥＬＥＣＴ 1",
            "\u{202E}1 TCELES",
            "/* unclosed SELECT 1",
        ] {
            let err = execute(proj.path(), sql, SandboxLimits::default())
                .expect_err(&format!("unicode/comment trick must be rejected: {sql:?}"));
            assert_eq!(err.rule, SandboxRule::NotSelect, "{sql:?}: {err:?}");
        }
    }

    #[test]
    fn instruction_budget_and_cancellation_stop_runaway_queries() {
        let proj = fixture_projection();
        let limits = SandboxLimits {
            timeout: Duration::from_secs(3),
            max_instructions: 50_000,
            ..SandboxLimits::default()
        };
        let started = Instant::now();
        let err = execute(
            proj.path(),
            "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt) SELECT COUNT(*) FROM cnt",
            limits,
        )
        .expect_err("instruction budget must stop runaway CTE");
        assert_eq!(err.rule, SandboxRule::InstructionLimit, "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(1));

        // Pre-cancelled token rejects immediately.
        let pre_cancel = SandboxCancel::new();
        pre_cancel.cancel();
        let err = execute_with_cancel(
            proj.path(),
            "SELECT COUNT(*) FROM listens",
            SandboxLimits::default(),
            Some(&pre_cancel),
        )
        .unwrap_err();
        assert_eq!(err.rule, SandboxRule::Cancelled, "{err:?}");

        // Live cancellation interrupts an in-flight query well before wall-clock timeout.
        let cancel = SandboxCancel::new();
        let trigger = cancel.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(25));
            trigger.cancel();
        });
        let started = Instant::now();
        let err = execute_with_cancel(
            proj.path(),
            "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt) SELECT COUNT(*) FROM cnt",
            SandboxLimits {
                timeout: Duration::from_secs(5),
                max_instructions: u64::MAX,
                ..SandboxLimits::default()
            },
            Some(&cancel),
        )
        .unwrap_err();
        handle.join().unwrap();
        assert_eq!(err.rule, SandboxRule::Cancelled, "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn output_byte_cap_truncates_multi_row_and_rejects_oversized_values_and_blobs() {
        let proj = fixture_projection();
        // Multi-row output exceeding max_bytes truncates cleanly after fitting rows.
        let limits = SandboxLimits {
            max_rows: 100,
            max_bytes: 16,
            ..SandboxLimits::default()
        };
        let out = execute(
            proj.path(),
            "WITH RECURSIVE cnt(s) AS (SELECT '1234567890' UNION ALL SELECT s FROM cnt LIMIT 5) SELECT s FROM cnt",
            limits,
        )
        .expect("multi-row byte cap should truncate rather than fail when at least one row fits");
        assert_eq!(out.rows.len(), 1);
        assert!(out.truncated);

        // Single row exceeding max_bytes rejects with OutputTooLarge.
        let err = execute(
            proj.path(),
            "SELECT '12345678901234567890'",
            SandboxLimits {
                max_bytes: 10,
                ..SandboxLimits::default()
            },
        )
        .unwrap_err();
        assert_eq!(err.rule, SandboxRule::OutputTooLarge, "{err:?}");

        // Concatenation exceeding SQLITE_LIMIT_LENGTH hits SQLITE_TOOBIG -> OutputTooLarge.
        let err = execute(
            proj.path(),
            "SELECT printf('%900000s', 'a') || printf('%900000s', 'a')",
            SandboxLimits::default(),
        )
        .unwrap_err();
        assert_eq!(err.rule, SandboxRule::OutputTooLarge, "{err:?}");

        // Huge printf width is capped by SQLITE_LIMIT_LENGTH (either errors or yields NULL without allocating).
        match execute(
            proj.path(),
            "SELECT printf('%10000000s', 'a')",
            SandboxLimits::default(),
        ) {
            Err(err) => assert_eq!(err.rule, SandboxRule::OutputTooLarge, "{err:?}"),
            Ok(out) => assert_eq!(out.rows, vec![vec![SqlValue::Null]]),
        }

        // BLOB values are rejected.
        for sql in ["SELECT X'deadbeef'", "SELECT CAST('abc' AS BLOB)"] {
            let err = execute(proj.path(), sql, SandboxLimits::default()).unwrap_err();
            assert_eq!(err.rule, SandboxRule::BlobNotAllowed, "{sql}: {err:?}");
        }
    }

    #[test]
    fn sanitized_errors_name_rule_for_owner_and_never_leak_projection_values() {
        let file = NamedTempFile::new().unwrap();
        let conn = Connection::open(file.path()).unwrap();
        conn.execute_batch(&projection::ddl()).unwrap();
        conn.execute(
            "INSERT INTO artists(id, name) VALUES ('a1', 'CanaryArtistSecret99')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO albums(id, name) VALUES ('al1', 'CanaryAlbumSecret99')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tracks(id, title, album_id, duration_ms, edition_key) VALUES ('t1', 'CanaryBrokenJson{\"unclosed', 'al1', 180000, 'canary:a1')",
            [],
        )
        .unwrap();

        let secrets = [
            "CanaryArtistSecret99",
            "CanaryAlbumSecret99",
            "CanaryBrokenJson",
            "unclosed",
        ];

        // Runtime errors triggered while reading projection rows must never leak row values.
        for (sql, limits, expected_rule) in [
            (
                "SELECT json(title) FROM tracks",
                SandboxLimits::default(),
                SandboxRule::SqlError,
            ),
            (
                "SELECT nth_value(title, 0) OVER () FROM tracks",
                SandboxLimits::default(),
                SandboxRule::SqlError,
            ),
            (
                "SELECT CAST(title AS BLOB) FROM tracks",
                SandboxLimits::default(),
                SandboxRule::BlobNotAllowed,
            ),
            (
                "SELECT title FROM tracks",
                SandboxLimits {
                    max_bytes: 5,
                    ..SandboxLimits::default()
                },
                SandboxRule::OutputTooLarge,
            ),
        ] {
            let err = execute(file.path(), sql, limits).unwrap_err();
            assert_eq!(err.rule, expected_rule, "{sql}: {err:?}");
            assert!(
                err.message.contains(err.rule_name()),
                "owner message must name rule `{}`: {}",
                err.rule_name(),
                err.message
            );
            for secret in secrets {
                assert!(
                    !err.model_error.contains(secret)
                        && !err.message.contains(secret)
                        && !err.detail.contains(secret),
                    "error leaked a projection value for `{sql}`"
                );
            }
        }

        // Prepare-time errors carry error class + offending identifier from the model's SQL.
        let err = execute(
            file.path(),
            "SELECT phantom_column FROM tracks",
            SandboxLimits::default(),
        )
        .unwrap_err();
        assert_eq!(err.rule, SandboxRule::SqlError);
        assert!(err.message.contains(err.rule_name()), "{err:?}");
        assert!(
            err.model_error.contains("phantom_column"),
            "expected offending identifier in model_error: {err:?}"
        );

        let err = execute(
            file.path(),
            "SELECT * FROM phantom_table",
            SandboxLimits::default(),
        )
        .unwrap_err();
        assert_eq!(err.rule, SandboxRule::UnauthorizedTable);
        assert!(err.message.contains(err.rule_name()), "{err:?}");
        assert!(
            err.model_error.contains("phantom_table"),
            "expected offending table in model_error: {err:?}"
        );

        let err = execute(
            file.path(),
            "SELECT phantom_func(id) FROM tracks",
            SandboxLimits::default(),
        )
        .unwrap_err();
        assert_eq!(err.rule, SandboxRule::SqlError);
        assert!(err.message.contains(err.rule_name()), "{err:?}");
        assert!(
            err.model_error.contains("phantom_func"),
            "expected offending function in model_error: {err:?}"
        );

        let err = execute(
            file.path(),
            "SELECT sqlite_version()",
            SandboxLimits::default(),
        )
        .unwrap_err();
        assert_eq!(err.rule, SandboxRule::UnauthorizedAction);
        assert!(err.message.contains(err.rule_name()), "{err:?}");
        assert!(
            err.model_error.contains("sqlite_version"),
            "expected disallowed function in model_error: {err:?}"
        );
    }

    #[test]
    fn unicode_case_and_accent_folding_work_in_sandbox() {
        let proj = fixture_projection();
        let out = execute(
            proj.path(),
            "SELECT name FROM artists WHERE LOWER(name) = LOWER('BJÖRK')",
            SandboxLimits::default(),
        )
        .expect("Unicode LOWER must match Björk");
        assert_eq!(out.rows, vec![vec![SqlValue::Text("Björk".into())]]);

        let out_upper = execute(
            proj.path(),
            "SELECT UPPER(name) FROM artists WHERE id = 'a1'",
            SandboxLimits::default(),
        )
        .expect("Unicode UPPER must convert Björk");
        assert_eq!(out_upper.rows, vec![vec![SqlValue::Text("BJÖRK".into())]]);

        let out_fold = execute(
            proj.path(),
            "SELECT name FROM artists WHERE fold(name) = fold('bjork')",
            SandboxLimits::default(),
        )
        .expect("fold() must match Björk and bjork");
        assert_eq!(out_fold.rows, vec![vec![SqlValue::Text("Björk".into())]]);
    }
}

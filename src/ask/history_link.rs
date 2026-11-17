//! Maps simple Ask SQL queries to exact `/history?...` filter URLs.
//!
//! Any query whose filters cannot be represented losslessly by `/history`
//! (such as `local_hour`, `OR`, `LIKE`, `HAVING`, `WITH`, subqueries, or
//! conflicting predicates) returns `None` rather than approximating.
use crate::ask::{
    SqlColumn,
    sandbox::SqlValue,
    schema::{ColumnFormat, SqlAnswerKind},
};
use chrono::{Days, NaiveDate};
use rusqlite::{Connection, OpenFlags};
use std::{collections::HashMap, fmt::Write as _, path::Path};

/// Open the projection for the fixed history-link queries. These queries are
/// app-generated, bind every parameter and only read; the SQL sandbox is the
/// path for model-written SQL, not this one.
fn open_projection_read_only(projection_path: &Path) -> Option<Connection> {
    let conn = Connection::open_with_flags(
        projection_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    // Belt and braces: even a future mistake here cannot write.
    let _ = conn.pragma_update(None, "query_only", true);
    Some(conn)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Str(String),
    Number(String),
    Op(String),
    LParen,
    RParen,
    Comma,
    Dot,
    Semi,
    Other,
}

fn tokenize(sql: &str) -> Option<Vec<Token>> {
    let bytes = sql.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'-' && bytes.get(i + 1) == Some(&b'-') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 >= bytes.len() {
                return None;
            }
            i += 2;
            continue;
        }
        match b {
            b'(' => {
                out.push(Token::LParen);
                i += 1;
            }
            b')' => {
                out.push(Token::RParen);
                i += 1;
            }
            b',' => {
                out.push(Token::Comma);
                i += 1;
            }
            b'.' => {
                out.push(Token::Dot);
                i += 1;
            }
            b';' => {
                out.push(Token::Semi);
                i += 1;
            }
            b'\'' => {
                i += 1;
                let mut text = String::new();
                while i < bytes.len() {
                    if bytes[i] == b'\'' {
                        if bytes.get(i + 1) == Some(&b'\'') {
                            text.push('\'');
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    let ch = sql[i..].chars().next()?;
                    text.push(ch);
                    i += ch.len_utf8();
                }
                if bytes.get(i) != Some(&b'\'') {
                    return None;
                }
                i += 1;
                out.push(Token::Str(text));
            }
            b'"' | b'`' => {
                let quote = b;
                i += 1;
                let mut ident = String::new();
                while i < bytes.len() {
                    if bytes[i] == quote {
                        if bytes.get(i + 1) == Some(&quote) {
                            ident.push(char::from(quote));
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    let ch = sql[i..].chars().next()?;
                    ident.push(ch);
                    i += ch.len_utf8();
                }
                if bytes.get(i) != Some(&quote) {
                    return None;
                }
                i += 1;
                out.push(Token::Word(ident));
            }
            b'[' => {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != b']' {
                    i += 1;
                }
                if bytes.get(i) != Some(&b']') {
                    return None;
                }
                out.push(Token::Word(sql[start..i].to_owned()));
                i += 1;
            }
            b'>' | b'<' | b'!' | b'=' => {
                let two = bytes.get(i + 1).copied();
                let op = match (b, two) {
                    (b'>', Some(b'=')) => {
                        i += 2;
                        ">="
                    }
                    (b'<', Some(b'=')) => {
                        i += 2;
                        "<="
                    }
                    (b'<', Some(b'>')) => {
                        i += 2;
                        "<>"
                    }
                    (b'!', Some(b'=')) => {
                        i += 2;
                        "!="
                    }
                    (b'>', _) => {
                        i += 1;
                        ">"
                    }
                    (b'<', _) => {
                        i += 1;
                        "<"
                    }
                    (b'=', _) => {
                        i += 1;
                        "="
                    }
                    _ => return None,
                };
                out.push(Token::Op(op.to_owned()));
            }
            c if c.is_ascii_digit() => {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                out.push(Token::Number(sql[start..i].to_owned()));
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                out.push(Token::Word(sql[start..i].to_owned()));
            }
            b'*' | b'+' | b'-' | b'/' | b'%' | b'|' | b'&' | b'~' => {
                out.push(Token::Other);
                i += 1;
            }
            _ => return None,
        }
    }
    Some(out)
}

fn is_word(tok: Option<&Token>, expected: &str) -> bool {
    matches!(tok, Some(Token::Word(w)) if w.eq_ignore_ascii_case(expected))
}

fn word_str(tok: Option<&Token>) -> Option<&str> {
    match tok {
        Some(Token::Word(w)) => Some(w.as_str()),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HistoryStatus {
    Counted,
    Short,
    Actual,
    Review,
    Estimated,
}

impl HistoryStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Counted => "counted",
            Self::Short => "short",
            Self::Actual => "actual",
            Self::Review => "review",
            Self::Estimated => "estimated",
        }
    }
}

#[derive(Default)]
struct ParsedFilters {
    status: Option<HistoryStatus>,
    year: Option<i32>,
    month: Option<u8>,
    weekday: Option<&'static str>,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    artist: Option<String>,
    album: Option<String>,
    track: Option<String>,
}

fn weekday_name(day: i64) -> Option<&'static str> {
    match day {
        1 => Some("monday"),
        2 => Some("tuesday"),
        3 => Some("wednesday"),
        4 => Some("thursday"),
        5 => Some("friday"),
        6 => Some("saturday"),
        7 => Some("sunday"),
        _ => None,
    }
}

fn parse_column(tokens: &[Token], aliases: &HashMap<String, String>) -> Option<(String, String)> {
    match tokens {
        [Token::Word(table), Token::Dot, Token::Word(col)] => {
            let canonical = aliases.get(&table.to_ascii_lowercase())?.clone();
            Some((canonical, col.to_ascii_lowercase()))
        }
        [Token::Word(col)] => {
            let lower = col.to_ascii_lowercase();
            match lower.as_str() {
                "counted" | "local_year" | "local_month" | "weekday" | "local_date"
                | "duration_kind" | "needs_review" => aliases
                    .values()
                    .any(|t| t == "listens")
                    .then(|| ("listens".into(), lower)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_from_and_joins(tokens: &[Token]) -> Option<(HashMap<String, String>, bool)> {
    let mut i = 0;
    let mut aliases = HashMap::new();
    let mut inner_albums = false;
    let table = word_str(tokens.get(i))?.to_ascii_lowercase();
    if !matches!(
        table.as_str(),
        "listens" | "tracks" | "albums" | "track_artists" | "artists"
    ) {
        return None;
    }
    i += 1;
    let alias = if is_word(tokens.get(i), "AS") {
        i += 1;
        let a = word_str(tokens.get(i))?.to_ascii_lowercase();
        i += 1;
        a
    } else if let Some(w) = word_str(tokens.get(i))
        && !matches!(
            w.to_ascii_uppercase().as_str(),
            "JOIN" | "INNER" | "LEFT" | "CROSS" | "RIGHT" | "FULL" | "NATURAL"
        )
    {
        i += 1;
        w.to_ascii_lowercase()
    } else {
        table.clone()
    };
    aliases.insert(alias, table);

    while i < tokens.len() {
        let is_left = if is_word(tokens.get(i), "LEFT") {
            i += 1;
            if is_word(tokens.get(i), "OUTER") {
                i += 1;
            }
            true
        } else {
            if is_word(tokens.get(i), "INNER") {
                i += 1;
            }
            false
        };
        if !is_word(tokens.get(i), "JOIN") {
            return None;
        }
        i += 1;
        let jtable = word_str(tokens.get(i))?.to_ascii_lowercase();
        if !matches!(
            jtable.as_str(),
            "listens" | "tracks" | "albums" | "track_artists" | "artists"
        ) {
            return None;
        }
        i += 1;
        let jalias = if is_word(tokens.get(i), "AS") {
            i += 1;
            let a = word_str(tokens.get(i))?.to_ascii_lowercase();
            i += 1;
            a
        } else if let Some(w) = word_str(tokens.get(i))
            && !w.eq_ignore_ascii_case("ON")
        {
            i += 1;
            w.to_ascii_lowercase()
        } else {
            jtable.clone()
        };
        aliases.insert(jalias, jtable.clone());

        if !is_word(tokens.get(i), "ON") {
            return None;
        }
        i += 1;
        let mut on_end = i;
        while on_end < tokens.len()
            && !is_word(tokens.get(on_end), "JOIN")
            && !is_word(tokens.get(on_end), "INNER")
            && !is_word(tokens.get(on_end), "LEFT")
        {
            on_end += 1;
        }
        let on_tokens = &tokens[i..on_end];
        let eq_pos = on_tokens
            .iter()
            .position(|t| matches!(t, Token::Op(op) if op == "="))?;
        let left = parse_column(&on_tokens[..eq_pos], &aliases)?;
        let right = parse_column(&on_tokens[eq_pos + 1..], &aliases)?;
        let valid_fk = matches!(
            (
                left.0.as_str(),
                left.1.as_str(),
                right.0.as_str(),
                right.1.as_str()
            ),
            ("listens", "track_id", "tracks", "id")
                | ("tracks", "id", "listens", "track_id")
                | ("listens", "track_id", "track_artists", "track_id")
                | ("track_artists", "track_id", "listens", "track_id")
                | ("tracks", "id", "track_artists", "track_id")
                | ("track_artists", "track_id", "tracks", "id")
                | ("tracks", "album_id", "albums", "id")
                | ("albums", "id", "tracks", "album_id")
                | ("track_artists", "artist_id", "artists", "id")
                | ("artists", "id", "track_artists", "artist_id")
        );
        if !valid_fk {
            return None;
        }
        if jtable == "albums" && !is_left {
            inner_albums = true;
        }
        i = on_end;
    }

    if !aliases.values().any(|t| t == "listens") {
        return None;
    }
    Some((aliases, inner_albums))
}

fn split_top_level_and(tokens: &[Token]) -> Option<Vec<&[Token]>> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < tokens.len() {
        match &tokens[i] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.checked_sub(1)?,
            Token::Word(w) if depth == 0 && w.eq_ignore_ascii_case("OR") => return None,
            Token::Word(w) if depth == 0 && w.eq_ignore_ascii_case("AND") => {
                // Keep `BETWEEN x AND y` intact as one predicate.
                let slice = &tokens[start..i];
                let has_between = slice.iter().any(|t| is_word(Some(t), "BETWEEN"));
                if !has_between {
                    parts.push(slice);
                    start = i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if depth != 0 {
        return None;
    }
    parts.push(&tokens[start..]);
    Some(parts)
}

fn parse_date_expr(tokens: &[Token]) -> Option<NaiveDate> {
    match tokens {
        [Token::Str(s)] => NaiveDate::parse_from_str(s, "%Y-%m-%d").ok(),
        [
            Token::Word(fn_name),
            Token::LParen,
            Token::Str(base),
            Token::Comma,
            Token::Str(mod_str),
            Token::RParen,
        ] if fn_name.eq_ignore_ascii_case("date") => {
            let base_date = NaiveDate::parse_from_str(base, "%Y-%m-%d").ok()?;
            let trimmed = mod_str.trim();
            let num_part = trimmed
                .strip_suffix("days")
                .or_else(|| trimmed.strip_suffix("day"))?
                .trim();
            let offset: i64 = num_part.parse().ok()?;
            if offset >= 0 {
                base_date.checked_add_days(Days::new(offset as u64))
            } else {
                base_date.checked_sub_days(Days::new(offset.unsigned_abs()))
            }
        }
        _ => None,
    }
}

fn strip_outer_parens(mut tokens: &[Token]) -> &[Token] {
    while tokens.len() >= 2
        && matches!(tokens.first(), Some(Token::LParen))
        && matches!(tokens.last(), Some(Token::RParen))
    {
        let mut depth = 0usize;
        let mut wraps_all = true;
        for (idx, tok) in tokens.iter().enumerate() {
            match tok {
                Token::LParen => depth += 1,
                Token::RParen => {
                    depth -= 1;
                    if depth == 0 && idx + 1 < tokens.len() {
                        wraps_all = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if wraps_all {
            tokens = &tokens[1..tokens.len() - 1];
        } else {
            break;
        }
    }
    tokens
}

fn parse_lower_arg(tokens: &[Token]) -> Option<&[Token]> {
    match tokens {
        [
            Token::Word(fn_name),
            Token::LParen,
            inner @ ..,
            Token::RParen,
        ] if fn_name.eq_ignore_ascii_case("LOWER") => Some(inner),
        _ => None,
    }
}

fn lookup_unique(conn: &Connection, sql: &str, value: &str) -> Option<String> {
    let mut stmt = conn.prepare_cached(sql).ok()?;
    let mut rows = stmt.query([value]).ok()?;
    let first: String = rows.next().ok()??.get(0).ok()?;
    if rows.next().ok()?.is_some() {
        return None;
    }
    Some(first)
}

fn apply_predicate(
    tokens: &[Token],
    aliases: &HashMap<String, String>,
    conn: &Connection,
    out: &mut ParsedFilters,
) -> Option<()> {
    let tokens = strip_outer_parens(tokens);

    // Handle `col BETWEEN expr1 AND expr2`.
    if let Some(between_idx) = tokens.iter().position(|t| is_word(Some(t), "BETWEEN")) {
        let and_idx = tokens[between_idx + 1..]
            .iter()
            .position(|t| is_word(Some(t), "AND"))
            .map(|rel| between_idx + 1 + rel)?;
        let col = parse_column(&tokens[..between_idx], aliases)?;
        if col != ("listens".into(), "local_date".into()) {
            return None;
        }
        let d1 = parse_date_expr(&tokens[between_idx + 1..and_idx])?;
        let d2 = parse_date_expr(&tokens[and_idx + 1..])?;
        out.from = Some(out.from.map_or(d1, |prev| prev.max(d1)));
        out.to = Some(out.to.map_or(d2, |prev| prev.min(d2)));
        return Some(());
    }

    // Handle `duration_kind IN ('estimated', 'unknown')`.
    if let Some(in_idx) = tokens.iter().position(|t| is_word(Some(t), "IN")) {
        let col = parse_column(&tokens[..in_idx], aliases)?;
        if col != ("listens".into(), "duration_kind".into()) {
            return None;
        }
        let rest = &tokens[in_idx + 1..];
        let [
            Token::LParen,
            Token::Str(a),
            Token::Comma,
            Token::Str(b),
            Token::RParen,
        ] = rest
        else {
            return None;
        };
        let mut pair = [a.as_str(), b.as_str()];
        pair.sort_unstable();
        if pair != ["estimated", "unknown"] {
            return None;
        }
        if out.status.is_some() {
            return None;
        }
        out.status = Some(HistoryStatus::Estimated);
        return Some(());
    }

    // Standard binary comparison: `lhs op rhs`.
    let op_pos = tokens.iter().position(|t| matches!(t, Token::Op(_)))?;
    let Token::Op(op) = &tokens[op_pos] else {
        return None;
    };
    let lhs = &tokens[..op_pos];
    let rhs = &tokens[op_pos + 1..];

    // Check LOWER(col) = LOWER('literal') or LOWER(col) = 'literal'
    if op == "="
        && let Some(inner_lhs) = parse_lower_arg(lhs)
    {
        let col = parse_column(inner_lhs, aliases)?;
        let value = match rhs {
            [Token::Str(s)] => s.as_str(),
            _ => match parse_lower_arg(rhs) {
                Some([Token::Str(s)]) => s.as_str(),
                _ => return None,
            },
        };
        return apply_entity_name(&col, value, true, conn, out);
    }

    // Allow `col op literal` or `literal op col`
    let (col, value_tokens, flipped) = if let Some(c) = parse_column(lhs, aliases) {
        (c, rhs, false)
    } else {
        let c = parse_column(rhs, aliases)?;
        (c, lhs, true)
    };

    let effective_op = if flipped {
        match op.as_str() {
            ">" => "<",
            "<" => ">",
            ">=" => "<=",
            "<=" => ">=",
            "=" => "=",
            _ => return None,
        }
    } else {
        op.as_str()
    };

    match (col.0.as_str(), col.1.as_str(), effective_op) {
        ("listens", "counted", "=") => {
            let [Token::Number(n)] = value_tokens else {
                return None;
            };
            if out.status.is_some() {
                return None;
            }
            out.status = Some(match n.as_str() {
                "1" => HistoryStatus::Counted,
                "0" => HistoryStatus::Short,
                _ => return None,
            });
            Some(())
        }
        ("listens", "duration_kind", "=") => {
            let [Token::Str(s)] = value_tokens else {
                return None;
            };
            if s != "actual" || out.status.is_some() {
                return None;
            }
            out.status = Some(HistoryStatus::Actual);
            Some(())
        }
        ("listens", "needs_review", "=") => {
            let [Token::Number(n)] = value_tokens else {
                return None;
            };
            if n != "1" || out.status.is_some() {
                return None;
            }
            out.status = Some(HistoryStatus::Review);
            Some(())
        }
        ("listens", "local_year", "=") => {
            let [Token::Number(n)] = value_tokens else {
                return None;
            };
            let year: i32 = n.parse().ok()?;
            if !(1..=9999).contains(&year) || out.year.is_some() {
                return None;
            }
            out.year = Some(year);
            Some(())
        }
        ("listens", "local_month", "=") => {
            let [Token::Number(n)] = value_tokens else {
                return None;
            };
            let month: u8 = n.parse().ok()?;
            if !(1..=12).contains(&month) || out.month.is_some() {
                return None;
            }
            out.month = Some(month);
            Some(())
        }
        ("listens", "weekday", "=") => {
            let [Token::Number(n)] = value_tokens else {
                return None;
            };
            let day: i64 = n.parse().ok()?;
            let name = weekday_name(day)?;
            if out.weekday.is_some() {
                return None;
            }
            out.weekday = Some(name);
            Some(())
        }
        ("listens", "local_date", op @ ("=" | ">=" | ">" | "<=" | "<")) => {
            let date = parse_date_expr(value_tokens)?;
            match op {
                "=" => {
                    out.from = Some(out.from.map_or(date, |prev| prev.max(date)));
                    out.to = Some(out.to.map_or(date, |prev| prev.min(date)));
                }
                ">=" => out.from = Some(out.from.map_or(date, |prev| prev.max(date))),
                ">" => {
                    let next = date.checked_add_days(Days::new(1))?;
                    out.from = Some(out.from.map_or(next, |prev| prev.max(next)));
                }
                "<=" => out.to = Some(out.to.map_or(date, |prev| prev.min(date))),
                "<" => {
                    let prev_day = date.checked_sub_days(Days::new(1))?;
                    out.to = Some(out.to.map_or(prev_day, |prev| prev.min(prev_day)));
                }
                _ => return None,
            }
            Some(())
        }
        ("artists", "id", "=") => {
            let [Token::Str(id)] = value_tokens else {
                return None;
            };
            if out.artist.is_some() {
                return None;
            }
            out.artist = Some(lookup_unique(
                conn,
                "SELECT id FROM artists WHERE id = ?1 LIMIT 1",
                id,
            )?);
            Some(())
        }
        ("albums", "id", "=") => {
            let [Token::Str(id)] = value_tokens else {
                return None;
            };
            if out.album.is_some() {
                return None;
            }
            out.album = Some(lookup_unique(
                conn,
                "SELECT id FROM albums WHERE id = ?1 LIMIT 1",
                id,
            )?);
            Some(())
        }
        ("tracks", "id", "=") => {
            let [Token::Str(id)] = value_tokens else {
                return None;
            };
            if out.track.is_some() {
                return None;
            }
            out.track = Some(lookup_unique(
                conn,
                "SELECT id FROM tracks WHERE id = ?1 LIMIT 1",
                id,
            )?);
            Some(())
        }
        (_, _, "=") => {
            let [Token::Str(val)] = value_tokens else {
                return None;
            };
            apply_entity_name(&col, val, false, conn, out)
        }
        _ => None,
    }
}

fn apply_entity_name(
    col: &(String, String),
    value: &str,
    case_insensitive: bool,
    conn: &Connection,
    out: &mut ParsedFilters,
) -> Option<()> {
    match (col.0.as_str(), col.1.as_str()) {
        ("artists", "name") => {
            if out.artist.is_some() {
                return None;
            }
            let sql = if case_insensitive {
                "SELECT id FROM artists WHERE LOWER(name) = LOWER(?1) LIMIT 2"
            } else {
                "SELECT id FROM artists WHERE name = ?1 LIMIT 2"
            };
            out.artist = Some(lookup_unique(conn, sql, value)?);
            Some(())
        }
        ("albums", "name") => {
            if out.album.is_some() {
                return None;
            }
            let sql = if case_insensitive {
                "SELECT id FROM albums WHERE LOWER(name) = LOWER(?1) LIMIT 2"
            } else {
                "SELECT id FROM albums WHERE name = ?1 LIMIT 2"
            };
            out.album = Some(lookup_unique(conn, sql, value)?);
            Some(())
        }
        ("tracks", "title") => {
            if out.track.is_some() {
                return None;
            }
            let sql = if case_insensitive {
                "SELECT id FROM tracks WHERE LOWER(title) = LOWER(?1) LIMIT 2"
            } else {
                "SELECT id FROM tracks WHERE title = ?1 LIMIT 2"
            };
            out.track = Some(lookup_unique(conn, sql, value)?);
            Some(())
        }
        _ => None,
    }
}

fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Map `sql` to an exact `/history?...` URL, or return `None` when the query's
/// filters cannot be represented without approximation.
pub fn resolve(projection_path: &Path, sql: &str, kind: SqlAnswerKind) -> Option<String> {
    if kind == SqlAnswerKind::Unsupported {
        return None;
    }
    let mut tokens = tokenize(sql)?;
    if matches!(tokens.last(), Some(Token::Semi)) {
        tokens.pop();
    }
    if tokens.is_empty() || !is_word(tokens.first(), "SELECT") {
        return None;
    }

    // Find top-level FROM, WHERE, GROUP, HAVING, ORDER, LIMIT, UNION/INTERSECT/EXCEPT.
    let mut depth = 0usize;
    let mut from_idx = None;
    let mut where_idx = None;
    let mut group_idx = None;
    let mut order_idx = None;
    let mut limit_idx = None;
    for (idx, tok) in tokens.iter().enumerate() {
        match tok {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.checked_sub(1)?,
            Token::Word(w) if depth == 0 => {
                if w.eq_ignore_ascii_case("UNION")
                    || w.eq_ignore_ascii_case("INTERSECT")
                    || w.eq_ignore_ascii_case("EXCEPT")
                    || w.eq_ignore_ascii_case("HAVING")
                    || w.eq_ignore_ascii_case("WINDOW")
                {
                    return None;
                }
                if w.eq_ignore_ascii_case("FROM") && from_idx.is_none() {
                    from_idx = Some(idx);
                } else if w.eq_ignore_ascii_case("WHERE") && where_idx.is_none() {
                    where_idx = Some(idx);
                } else if w.eq_ignore_ascii_case("GROUP") && group_idx.is_none() {
                    group_idx = Some(idx);
                } else if w.eq_ignore_ascii_case("ORDER") && order_idx.is_none() {
                    order_idx = Some(idx);
                } else if w.eq_ignore_ascii_case("LIMIT") && limit_idx.is_none() {
                    limit_idx = Some(idx);
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }

    let from_start = from_idx? + 1;
    let from_end = [where_idx, group_idx, order_idx, limit_idx]
        .into_iter()
        .flatten()
        .filter(|&p| p >= from_start)
        .min()
        .unwrap_or(tokens.len());
    let (aliases, inner_albums) = parse_from_and_joins(&tokens[from_start..from_end])?;

    let mut filters = ParsedFilters::default();
    if let Some(w_idx) = where_idx {
        let w_start = w_idx + 1;
        let w_end = [group_idx, order_idx, limit_idx]
            .into_iter()
            .flatten()
            .filter(|&p| p >= w_start)
            .min()
            .unwrap_or(tokens.len());
        let where_tokens = &tokens[w_start..w_end];
        if where_tokens.is_empty() {
            return None;
        }
        let predicates = split_top_level_and(where_tokens)?;
        let conn = open_projection_read_only(projection_path)?;
        for pred in predicates {
            apply_predicate(pred, &aliases, &conn, &mut filters)?;
        }
    }

    if inner_albums && filters.album.is_none() {
        return None;
    }

    if let (Some(from), Some(to)) = (filters.from, filters.to)
        && from > to
    {
        return None;
    }
    if filters.year.is_some() && (filters.from.is_some() || filters.to.is_some()) {
        return None;
    }

    let mut params: Vec<(&'static str, String)> = Vec::new();
    if filters.from.is_some() || filters.to.is_some() {
        params.push(("period", "custom".into()));
        if let Some(from) = filters.from {
            params.push(("from", from.to_string()));
        }
        if let Some(to) = filters.to {
            params.push(("to", to.to_string()));
        }
    } else {
        params.push(("period", "all".into()));
        if let Some(year) = filters.year {
            params.push(("year", year.to_string()));
        }
    }
    if let Some(month) = filters.month {
        params.push(("month", month.to_string()));
    }
    if let Some(weekday) = filters.weekday {
        params.push(("weekday", weekday.into()));
    }
    if let Some(artist) = filters.artist {
        params.push(("artist", artist));
    }
    if let Some(album) = filters.album {
        params.push(("album", album));
    }
    if let Some(track) = filters.track {
        params.push(("track", track));
    }
    params.push((
        "status",
        filters
            .status
            .map_or("all", HistoryStatus::as_str)
            .to_owned(),
    ));

    let query = params
        .iter()
        .map(|(k, v)| format!("{k}={}", encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    Some(format!("/history?{query}"))
}

fn music_href(segment: &str, id: &str) -> String {
    format!("/music/{segment}/{}", encode(id))
}

fn text_at(row: &[SqlValue], col: Option<usize>) -> Option<&str> {
    match col.and_then(|idx| row.get(idx)) {
        Some(SqlValue::Text(s)) if !s.is_empty() => Some(s.as_str()),
        _ => None,
    }
}

fn unique_query(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Option<String> {
    let mut stmt = conn.prepare_cached(sql).ok()?;
    let mut rows = stmt.query(params).ok()?;
    let first: String = rows.next().ok()??.get(0).ok()?;
    if rows.next().ok()?.is_some() {
        return None;
    }
    Some(first)
}

/// Resolve `/music/{artists|albums|tracks}/{id}` links for `artist`, `album`,
/// and `track` columns using the projection's link keys. Ambiguous names without
/// an ID or disambiguating column in the same row stay unlinked (`None`).
pub fn resolve_cell_links(
    projection_path: &Path,
    columns: &[SqlColumn],
    rows: &[Vec<SqlValue>],
) -> Vec<Vec<Option<String>>> {
    if rows.is_empty()
        || !columns.iter().any(|c| {
            matches!(
                c.format,
                ColumnFormat::Artist | ColumnFormat::Album | ColumnFormat::Track
            )
        })
    {
        return Vec::new();
    }
    let Some(conn) = open_projection_read_only(projection_path) else {
        return Vec::new();
    };

    let entity_cols = columns
        .iter()
        .filter(|c| {
            matches!(
                c.format,
                ColumnFormat::Artist | ColumnFormat::Album | ColumnFormat::Track
            )
        })
        .count();
    let find_col = |name: &str| {
        columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
    };
    let id_col = find_col("id").filter(|_| entity_cols == 1);
    let artist_id_col = find_col("artist_id").or_else(|| {
        columns
            .iter()
            .any(|c| c.format == ColumnFormat::Artist)
            .then_some(id_col)
            .flatten()
    });
    let album_id_col = find_col("album_id").or_else(|| {
        columns
            .iter()
            .any(|c| c.format == ColumnFormat::Album)
            .then_some(id_col)
            .flatten()
    });
    let track_id_col = find_col("track_id").or_else(|| {
        columns
            .iter()
            .any(|c| c.format == ColumnFormat::Track)
            .then_some(id_col)
            .flatten()
    });
    let artist_name_col = columns
        .iter()
        .position(|c| c.format == ColumnFormat::Artist);
    let album_name_col = columns.iter().position(|c| c.format == ColumnFormat::Album);

    type LinkCacheKey = (u8, String, Option<String>, Option<String>);
    let mut id_cache: HashMap<(&'static str, String), Option<String>> = HashMap::new();
    let mut name_cache: HashMap<LinkCacheKey, Option<String>> = HashMap::new();

    rows.iter()
        .map(|row| {
            columns
                .iter()
                .enumerate()
                .map(|(idx, col)| {
                    let text = match row.get(idx) {
                        Some(SqlValue::Text(s)) if !s.is_empty() => s.as_str(),
                        _ => return None,
                    };
                    match col.format {
                        ColumnFormat::Artist => {
                            if let Some(id) = text_at(row, artist_id_col) {
                                return id_cache
                                    .entry(("artists", id.to_owned()))
                                    .or_insert_with(|| {
                                        unique_query(
                                            &conn,
                                            "SELECT id FROM artists WHERE id = ?1 LIMIT 1",
                                            &[&id],
                                        )
                                        .map(|found| music_href("artists", &found))
                                    })
                                    .clone();
                            }
                            let key = (0u8, text.to_owned(), None, None);
                            name_cache
                                .entry(key)
                                .or_insert_with(|| {
                                    unique_query(
                                        &conn,
                                        "SELECT id FROM artists WHERE name = ?1 LIMIT 2",
                                        &[&text],
                                    )
                                    .map(|found| music_href("artists", &found))
                                })
                                .clone()
                        }
                        ColumnFormat::Album => {
                            if let Some(id) = text_at(row, album_id_col) {
                                return id_cache
                                    .entry(("albums", id.to_owned()))
                                    .or_insert_with(|| {
                                        unique_query(
                                            &conn,
                                            "SELECT id FROM albums WHERE id = ?1 LIMIT 1",
                                            &[&id],
                                        )
                                        .map(|found| music_href("albums", &found))
                                    })
                                    .clone();
                            }
                            let artist = text_at(row, artist_name_col).map(str::to_owned);
                            let key = (1u8, text.to_owned(), artist.clone(), None);
                            name_cache
                                .entry(key)
                                .or_insert_with(|| {
                                    unique_query(
                                        &conn,
                                        "SELECT id FROM albums WHERE name = ?1 LIMIT 2",
                                        &[&text],
                                    )
                                    .or_else(|| {
                                        let ar = artist.as_deref()?;
                                        unique_query(
                                            &conn,
                                            "SELECT DISTINCT al.id FROM albums al \
                                             JOIN tracks t ON t.album_id = al.id \
                                             JOIN track_artists ta ON ta.track_id = t.id \
                                             JOIN artists ar ON ar.id = ta.artist_id \
                                             WHERE al.name = ?1 AND ar.name = ?2 LIMIT 2",
                                            &[&text, &ar],
                                        )
                                    })
                                    .map(|found| music_href("albums", &found))
                                })
                                .clone()
                        }
                        ColumnFormat::Track => {
                            if let Some(id) = text_at(row, track_id_col) {
                                return id_cache
                                    .entry(("tracks", id.to_owned()))
                                    .or_insert_with(|| {
                                        unique_query(
                                            &conn,
                                            "SELECT id FROM tracks WHERE id = ?1 LIMIT 1",
                                            &[&id],
                                        )
                                        .map(|found| music_href("tracks", &found))
                                    })
                                    .clone();
                            }
                            let artist = text_at(row, artist_name_col).map(str::to_owned);
                            let album = text_at(row, album_name_col).map(str::to_owned);
                            let key = (2u8, text.to_owned(), artist.clone(), album.clone());
                            name_cache
                                .entry(key)
                                .or_insert_with(|| {
                                    unique_query(
                                        &conn,
                                        "SELECT id FROM tracks WHERE title = ?1 LIMIT 2",
                                        &[&text],
                                    )
                                    .or_else(|| match (artist.as_deref(), album.as_deref()) {
                                        (Some(ar), Some(al)) => unique_query(
                                            &conn,
                                            "SELECT DISTINCT t.id FROM tracks t \
                                             JOIN albums al ON al.id = t.album_id \
                                             JOIN track_artists ta ON ta.track_id = t.id \
                                             JOIN artists ar ON ar.id = ta.artist_id \
                                             WHERE t.title = ?1 AND ar.name = ?2 AND al.name = ?3 LIMIT 2",
                                            &[&text, &ar, &al],
                                        ),
                                        (Some(ar), None) => unique_query(
                                            &conn,
                                            "SELECT DISTINCT t.id FROM tracks t \
                                             JOIN track_artists ta ON ta.track_id = t.id \
                                             JOIN artists ar ON ar.id = ta.artist_id \
                                             WHERE t.title = ?1 AND ar.name = ?2 LIMIT 2",
                                            &[&text, &ar],
                                        ),
                                        (None, Some(al)) => unique_query(
                                            &conn,
                                            "SELECT DISTINCT t.id FROM tracks t \
                                             JOIN albums al ON al.id = t.album_id \
                                             WHERE t.title = ?1 AND al.name = ?2 LIMIT 2",
                                            &[&text, &al],
                                        ),
                                        (None, None) => None,
                                    })
                                    .map(|found| music_href("tracks", &found))
                                })
                                .clone()
                        }
                        _ => None,
                    }
                })
                .collect()
        })
        .collect()
}

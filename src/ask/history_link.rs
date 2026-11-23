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
use rusqlite::Connection;
use std::{collections::HashMap, fmt::Write as _, path::Path};

/// Open the projection for the fixed history-link queries via the canonical
/// read-only projection opener.
fn open_projection_read_only(projection_path: &Path) -> Option<Connection> {
    crate::ask::projection::open_read_only(projection_path)
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

fn last_day_of_month(year: i32, month: u8) -> Option<NaiveDate> {
    let (next_year, next_month) = if month == 12 {
        (year.checked_add(1)?, 1)
    } else {
        (year, u32::from(month) + 1)
    };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)?.pred_opt()
}

fn normalize_year_month_bounds(filters: &mut ParsedFilters) -> Option<()> {
    if filters.year.is_some() && (filters.from.is_some() || filters.to.is_some()) {
        return None;
    }
    if let Some(year) = filters.year.take() {
        if let Some(month) = filters.month.take() {
            filters.from = Some(NaiveDate::from_ymd_opt(year, u32::from(month), 1)?);
            filters.to = Some(last_day_of_month(year, month)?);
        } else {
            filters.from = Some(NaiveDate::from_ymd_opt(year, 1, 1)?);
            filters.to = Some(NaiveDate::from_ymd_opt(year, 12, 31)?);
        }
    }
    Some(())
}

fn collect_loose_table_aliases(tokens: &[Token]) -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < tokens.len() {
        match &tokens[i] {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(w)
                if depth == 0
                    && (w.eq_ignore_ascii_case("FROM") || w.eq_ignore_ascii_case("JOIN")) =>
            {
                if let Some(table) = word_str(tokens.get(i + 1)) {
                    let canonical = table.to_ascii_lowercase();
                    if matches!(
                        canonical.as_str(),
                        "listens"
                            | "tracks"
                            | "albums"
                            | "track_artists"
                            | "album_artists"
                            | "artists"
                    ) {
                        let mut j = i + 2;
                        let alias = if is_word(tokens.get(j), "AS") {
                            j += 1;
                            word_str(tokens.get(j))
                                .map(str::to_ascii_lowercase)
                                .unwrap_or_else(|| canonical.clone())
                        } else if let Some(next_w) = word_str(tokens.get(j))
                            && !matches!(
                                next_w.to_ascii_uppercase().as_str(),
                                "ON" | "WHERE"
                                    | "GROUP"
                                    | "ORDER"
                                    | "LIMIT"
                                    | "JOIN"
                                    | "INNER"
                                    | "LEFT"
                                    | "CROSS"
                                    | "RIGHT"
                                    | "FULL"
                                    | "NATURAL"
                                    | "UNION"
                                    | "INTERSECT"
                                    | "EXCEPT"
                            )
                        {
                            next_w.to_ascii_lowercase()
                        } else {
                            canonical.clone()
                        };
                        aliases.insert(alias, canonical);
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    aliases
}

fn split_top_level_commas(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (idx, tok) in tokens.iter().enumerate() {
        match tok {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Comma if depth == 0 => {
                out.push(&tokens[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    if start < tokens.len() {
        out.push(&tokens[start..]);
    }
    out
}

fn strip_select_alias(tokens: &[Token]) -> &[Token] {
    if tokens.len() >= 3 && is_word(tokens.get(tokens.len() - 2), "AS") {
        if matches!(tokens.last(), Some(Token::Word(_))) {
            return &tokens[..tokens.len() - 2];
        }
    } else if tokens.len() >= 2
        && matches!(tokens.last(), Some(Token::Word(_)))
        && !matches!(
            tokens.get(tokens.len() - 2),
            Some(Token::Dot | Token::LParen)
        )
    {
        return &tokens[..tokens.len() - 1];
    }
    tokens
}

fn unwrap_min_max(tokens: &[Token]) -> &[Token] {
    match tokens {
        [
            Token::Word(fn_name),
            Token::LParen,
            inner @ ..,
            Token::RParen,
        ] if fn_name.eq_ignore_ascii_case("MIN") || fn_name.eq_ignore_ascii_case("MAX") => inner,
        _ => tokens,
    }
}

fn infer_entity_format(expr: &[Token], aliases: &HashMap<String, String>) -> Option<ColumnFormat> {
    let expr = strip_outer_parens(unwrap_min_max(strip_outer_parens(expr)));
    match expr {
        [Token::Word(tbl), Token::Dot, Token::Word(col)] => {
            let canonical = aliases.get(&tbl.to_ascii_lowercase())?;
            match (canonical.as_str(), col.to_ascii_lowercase().as_str()) {
                ("tracks", "title") => Some(ColumnFormat::Track),
                ("artists", "name") => Some(ColumnFormat::Artist),
                ("albums", "name") => Some(ColumnFormat::Album),
                _ => None,
            }
        }
        [Token::Word(col)] => {
            let lower = col.to_ascii_lowercase();
            let has_tracks = aliases.values().any(|t| t == "tracks");
            let has_artists = aliases.values().any(|t| t == "artists");
            let has_albums = aliases.values().any(|t| t == "albums");
            match lower.as_str() {
                "title" if has_tracks => Some(ColumnFormat::Track),
                "name" if has_artists && !has_albums => Some(ColumnFormat::Artist),
                "name" if has_albums && !has_artists => Some(ColumnFormat::Album),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Promote `ColumnFormat::Text` columns to `Track`/`Artist`/`Album` when the
/// SQL `SELECT` projects `tracks.title`, `artists.name`, or `albums.name`.
pub fn promote_entity_columns(sql: &str, columns: &mut [SqlColumn]) {
    let Some(mut tokens) = tokenize(sql) else {
        return;
    };
    if matches!(tokens.last(), Some(Token::Semi)) {
        tokens.pop();
    }
    let mut depth = 0usize;
    let mut select_idx = None;
    let mut from_idx = None;
    for (idx, tok) in tokens.iter().enumerate() {
        match tok {
            Token::LParen => depth += 1,
            Token::RParen => depth = depth.saturating_sub(1),
            Token::Word(w) if depth == 0 => {
                if w.eq_ignore_ascii_case("SELECT") && select_idx.is_none() {
                    select_idx = Some(idx);
                } else if w.eq_ignore_ascii_case("FROM") && select_idx.is_some() {
                    from_idx = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }
    let (Some(s_idx), Some(f_idx)) = (select_idx, from_idx) else {
        return;
    };
    let mut proj_start = s_idx + 1;
    if is_word(tokens.get(proj_start), "DISTINCT") {
        proj_start += 1;
    }
    if proj_start >= f_idx {
        return;
    }
    let aliases = collect_loose_table_aliases(&tokens[f_idx..]);
    let items = split_top_level_commas(&tokens[proj_start..f_idx]);
    if items.len() != columns.len() {
        return;
    }
    for (item, col) in items.iter().zip(columns.iter_mut()) {
        if col.format != ColumnFormat::Text {
            continue;
        }
        let expr = strip_select_alias(item);
        if let Some(promoted) = infer_entity_format(expr, &aliases) {
            col.format = promoted;
        }
    }
}

fn extract_select_alias_and_expr(tokens: &[Token]) -> (Option<String>, &[Token]) {
    if tokens.len() >= 3 && is_word(tokens.get(tokens.len() - 2), "AS") {
        if let Some(Token::Word(alias)) = tokens.last() {
            return (
                Some(alias.to_ascii_lowercase()),
                &tokens[..tokens.len() - 2],
            );
        }
    } else if tokens.len() >= 2
        && let Some(Token::Word(alias)) = tokens.last()
        && !matches!(
            tokens.get(tokens.len() - 2),
            Some(Token::Dot | Token::LParen)
        )
    {
        return (
            Some(alias.to_ascii_lowercase()),
            &tokens[..tokens.len() - 1],
        );
    }
    (None, tokens)
}

fn ranking_entity_segment(
    group_tokens: &[Token],
    aliases: &HashMap<String, String>,
) -> Option<&'static str> {
    let items = split_top_level_commas(group_tokens);
    if items.is_empty() {
        return None;
    }
    let mut segment: Option<&'static str> = None;
    for item in items {
        let (tbl, col) = parse_column(strip_outer_parens(item), aliases)?;
        let item_seg = match (tbl.as_str(), col.as_str()) {
            ("artists", "id" | "name") => "artists",
            ("albums", "id" | "name") => "albums",
            ("tracks", "id" | "edition_key" | "title") => "tracks",
            _ => return None,
        };
        if let Some(prev) = segment {
            if prev != item_seg {
                return None;
            }
        } else {
            segment = Some(item_seg);
        }
    }
    segment
}

fn classify_ranking_metric(tokens: &[Token]) -> Option<&'static str> {
    let expr = strip_outer_parens(tokens);
    if let [
        Token::Word(fn_name),
        Token::LParen,
        inner @ ..,
        Token::RParen,
    ] = expr
    {
        if fn_name.eq_ignore_ascii_case("COALESCE") {
            let args = split_top_level_commas(inner);
            if let Some(first) = args.first() {
                return classify_ranking_metric(first);
            }
            return None;
        }
        if fn_name.eq_ignore_ascii_case("COUNT") {
            return Some("plays");
        }
        if fn_name.eq_ignore_ascii_case("SUM") {
            let has_duration = inner.iter().any(|t| {
                matches!(
                    t,
                    Token::Word(w)
                        if w.eq_ignore_ascii_case("listened_ms")
                            || w.eq_ignore_ascii_case("duration_ms")
                )
            });
            if has_duration {
                return Some("time");
            }
        }
    }
    None
}

fn ranking_sort_order(
    order_tokens: Option<&[Token]>,
    select_tokens: &[Token],
    kind: SqlAnswerKind,
) -> Option<(&'static str, bool)> {
    let mut alias_map: HashMap<String, &[Token]> = HashMap::new();
    let select_items = split_top_level_commas(select_tokens);
    for item in &select_items {
        let (alias, expr) = extract_select_alias_and_expr(item);
        if let Some(name) = alias {
            alias_map.insert(name, expr);
        }
    }

    if let Some(ot) = order_tokens {
        if ot.len() < 3 || !is_word(ot.get(1), "BY") {
            return None;
        }
        let items = split_top_level_commas(&ot[2..]);
        let primary = items.first()?;
        let (expr, asc) = if is_word(primary.last(), "ASC") {
            (&primary[..primary.len() - 1], true)
        } else if is_word(primary.last(), "DESC") {
            (&primary[..primary.len() - 1], false)
        } else {
            (*primary, false)
        };
        let resolved = match strip_outer_parens(expr) {
            [Token::Word(w)] => alias_map
                .get(&w.to_ascii_lowercase())
                .copied()
                .unwrap_or(expr),
            _ => expr,
        };
        return classify_ranking_metric(resolved).map(|sort| (sort, asc));
    }

    if kind == SqlAnswerKind::Ranking {
        for item in &select_items {
            let (_, expr) = extract_select_alias_and_expr(item);
            if let Some(sort) = classify_ranking_metric(expr) {
                return Some((sort, false));
            }
        }
    }
    None
}

/// Map `sql` to an exact `/rankings/...` or `/history?...` URL, or return
/// `None` when the query's filters cannot be represented without approximation.
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

    normalize_year_month_bounds(&mut filters)?;
    if let (Some(from), Some(to)) = (filters.from, filters.to)
        && from > to
    {
        return None;
    }

    let ranking_compatible = filters.status == Some(HistoryStatus::Counted)
        && filters.month.is_none()
        && filters.weekday.is_none()
        && filters.artist.is_none()
        && filters.album.is_none()
        && filters.track.is_none();
    if ranking_compatible
        && let Some(g_idx) = group_idx
        && is_word(tokens.get(g_idx + 1), "BY")
    {
        let g_start = g_idx + 2;
        let g_end = [order_idx, limit_idx]
            .into_iter()
            .flatten()
            .filter(|&p| p >= g_start)
            .min()
            .unwrap_or(tokens.len());
        let order_tokens = order_idx.map(|o_idx| {
            let o_end = limit_idx.filter(|&p| p >= o_idx).unwrap_or(tokens.len());
            &tokens[o_idx..o_end]
        });
        let select_start = if is_word(tokens.get(1), "DISTINCT") {
            2
        } else {
            1
        };
        let select_tokens = &tokens[select_start..from_start - 1];
        if let Some(segment) = ranking_entity_segment(&tokens[g_start..g_end], &aliases)
            && let Some((sort, asc)) = ranking_sort_order(order_tokens, select_tokens, kind)
        {
            let mut r_params: Vec<(&'static str, String)> = Vec::new();
            if filters.from.is_some() || filters.to.is_some() {
                r_params.push(("period", "custom".into()));
                if let Some(from) = filters.from {
                    r_params.push(("from", from.to_string()));
                }
                if let Some(to) = filters.to {
                    r_params.push(("to", to.to_string()));
                }
            } else {
                r_params.push(("period", "all".into()));
            }
            r_params.push(("sort", sort.into()));
            if asc {
                r_params.push(("order", "asc".into()));
            }
            let query = r_params
                .iter()
                .map(|(k, v)| format!("{k}={}", encode(v)))
                .collect::<Vec<_>>()
                .join("&");
            return Some(format!("/rankings/{segment}?{query}"));
        }
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

fn extract_period_query(conn: &Connection, sql: &str) -> Option<String> {
    let mut tokens = tokenize(sql)?;
    if matches!(tokens.last(), Some(Token::Semi)) {
        tokens.pop();
    }
    if tokens.is_empty() || !is_word(tokens.first(), "SELECT") {
        return None;
    }
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
    let (aliases, _) = parse_from_and_joins(&tokens[from_start..from_end])?;
    let w_idx = where_idx?;
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
    let mut filters = ParsedFilters::default();
    for pred in predicates {
        apply_predicate(pred, &aliases, conn, &mut filters)?;
    }
    normalize_year_month_bounds(&mut filters)?;
    if let (Some(from), Some(to)) = (filters.from, filters.to)
        && from > to
    {
        return None;
    }

    if filters.from.is_none() && filters.to.is_none() {
        return None;
    }
    let mut params = vec![("period", "custom".to_string())];
    if let Some(from) = filters.from {
        params.push(("from", from.to_string()));
    }
    if let Some(to) = filters.to {
        params.push(("to", to.to_string()));
    }
    Some(
        params
            .iter()
            .map(|(k, v)| format!("{k}={}", encode(v)))
            .collect::<Vec<_>>()
            .join("&"),
    )
}

fn music_href(segment: &str, id: &str, period_query: Option<&str>) -> String {
    match period_query {
        Some(pq) => format!("/music/{segment}/{}?{pq}", encode(id)),
        None => format!("/music/{segment}/{}", encode(id)),
    }
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

fn resolve_track_edition(
    conn: &Connection,
    title: &str,
    artist: Option<&str>,
    album: Option<&str>,
) -> Option<String> {
    let (distinct_sql, pick_sql, params): (&str, &str, Vec<&dyn rusqlite::ToSql>) =
        match (&artist, &album) {
            (Some(ar), Some(al)) => (
                "SELECT COUNT(DISTINCT t.edition_key) FROM tracks t \
                 JOIN albums al ON al.id = t.album_id \
                 JOIN track_artists ta ON ta.track_id = t.id \
                 JOIN artists ar ON ar.id = ta.artist_id \
                 WHERE t.title = ?1 AND ar.name = ?2 AND al.name = ?3",
                "SELECT t.id FROM tracks t \
                 JOIN albums al ON al.id = t.album_id \
                 JOIN track_artists ta ON ta.track_id = t.id \
                 JOIN artists ar ON ar.id = ta.artist_id \
                 LEFT JOIN listens l ON l.track_id = t.id AND l.counted = 1 \
                 WHERE t.title = ?1 AND ar.name = ?2 AND al.name = ?3 \
                 GROUP BY t.id \
                 ORDER BY COUNT(l.id) DESC, t.id ASC \
                 LIMIT 1",
                vec![&title, ar, al],
            ),
            (Some(ar), None) => (
                "SELECT COUNT(DISTINCT t.edition_key) FROM tracks t \
                 JOIN track_artists ta ON ta.track_id = t.id \
                 JOIN artists ar ON ar.id = ta.artist_id \
                 WHERE t.title = ?1 AND ar.name = ?2",
                "SELECT t.id FROM tracks t \
                 JOIN track_artists ta ON ta.track_id = t.id \
                 JOIN artists ar ON ar.id = ta.artist_id \
                 LEFT JOIN listens l ON l.track_id = t.id AND l.counted = 1 \
                 WHERE t.title = ?1 AND ar.name = ?2 \
                 GROUP BY t.id \
                 ORDER BY COUNT(l.id) DESC, t.id ASC \
                 LIMIT 1",
                vec![&title, ar],
            ),
            (None, Some(al)) => (
                "SELECT COUNT(DISTINCT t.edition_key) FROM tracks t \
                 JOIN albums al ON al.id = t.album_id \
                 WHERE t.title = ?1 AND al.name = ?2",
                "SELECT t.id FROM tracks t \
                 JOIN albums al ON al.id = t.album_id \
                 LEFT JOIN listens l ON l.track_id = t.id AND l.counted = 1 \
                 WHERE t.title = ?1 AND al.name = ?2 \
                 GROUP BY t.id \
                 ORDER BY COUNT(l.id) DESC, t.id ASC \
                 LIMIT 1",
                vec![&title, al],
            ),
            (None, None) => (
                "SELECT COUNT(DISTINCT edition_key) FROM tracks WHERE title = ?1",
                "SELECT t.id FROM tracks t \
                 LEFT JOIN listens l ON l.track_id = t.id AND l.counted = 1 \
                 WHERE t.title = ?1 \
                 GROUP BY t.id \
                 ORDER BY COUNT(l.id) DESC, t.id ASC \
                 LIMIT 1",
                vec![&title],
            ),
        };
    let distinct_editions: i64 = conn
        .prepare_cached(distinct_sql)
        .ok()?
        .query_row(params.as_slice(), |r| r.get(0))
        .ok()?;
    if distinct_editions != 1 {
        return None;
    }
    conn.prepare_cached(pick_sql)
        .ok()?
        .query_row(params.as_slice(), |r| r.get(0))
        .ok()
}

/// Resolve `/music/{artists|albums|tracks}/{id}` links for `artist`, `album`,
/// and `track` columns using the projection's link keys. Ambiguous names without
/// an ID or disambiguating column in the same row stay unlinked (`None`).
pub fn resolve_cell_links(
    projection_path: &Path,
    sql: &str,
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
    let period_query = extract_period_query(&conn, sql);
    let pq = period_query.as_deref();

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
                                        .map(|found| music_href("artists", &found, pq))
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
                                    .map(|found| music_href("artists", &found, pq))
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
                                        .map(|found| music_href("albums", &found, pq))
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
                                    .map(|found| music_href("albums", &found, pq))
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
                                        .map(|found| music_href("tracks", &found, pq))
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
                                    .or_else(|| {
                                        resolve_track_edition(
                                            &conn,
                                            text,
                                            artist.as_deref(),
                                            album.as_deref(),
                                        )
                                    })
                                    .map(|found| music_href("tracks", &found, pq))
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

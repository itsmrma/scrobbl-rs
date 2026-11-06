//! Resolve an entity literal from the question against the existing library.
//!
//! The model never produces IDs. Matching is Unicode-safe: case and accent
//! folding keeps digits, apostrophes, `&` and CJK intact, and ambiguity always
//! becomes an explicit refusal with candidates instead of a guess.
use crate::{
    ask::{EntityCandidate, dsl::EntityKind},
    error::Result,
};
use sqlx::{FromRow, SqliteConnection};
use std::collections::BTreeMap;

#[derive(Debug, FromRow)]
struct NameRow {
    id: String,
    name: String,
    subtitle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Unique { id: String, name: String },
    Ambiguous(Vec<EntityCandidate>),
    NotFound,
}

/// Resolve a library entity by literal name. A unique match wins; otherwise the
/// caller must show the candidates (or refuse when nothing plausible exists).
pub async fn resolve(
    conn: &mut SqliteConnection,
    kind: EntityKind,
    literal: &str,
) -> Result<Resolution> {
    let needle = fold(literal);
    if needle.is_empty() {
        return Ok(Resolution::NotFound);
    }
    let mut rows = search_rows(conn, kind, literal).await?;
    if rows.is_empty() {
        // SQLite LIKE folds ASCII case only. Fall back to a Rust fold so
        // "Bjork" still finds "Björk" without a new dependency.
        rows = all_rows(conn, kind).await?;
        rows.retain(|row| fold(&row.name).contains(&needle));
    }
    if rows.is_empty() {
        return Ok(Resolution::NotFound);
    }
    let exact: Vec<NameRow> = rows
        .iter()
        .filter(|row| fold(&row.name) == needle)
        .map(cloned_row)
        .collect();
    let matched = if exact.is_empty() { rows } else { exact };
    let mut unique: BTreeMap<String, NameRow> = BTreeMap::new();
    for row in matched {
        unique.entry(row.id.clone()).or_insert(row);
    }
    let mut rows: Vec<NameRow> = unique.into_values().collect();
    rows.sort_by(|left, right| {
        fold(&left.name)
            .cmp(&fold(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    if rows.len() == 1 {
        let row = rows.remove(0);
        return Ok(Resolution::Unique {
            id: row.id,
            name: row.name,
        });
    }
    Ok(Resolution::Ambiguous(
        rows.into_iter()
            .take(10)
            .map(|row| EntityCandidate {
                id: row.id,
                name: row.name,
                subtitle: row.subtitle.unwrap_or_default(),
                kind,
            })
            .collect(),
    ))
}

/// Verify an explicit ID (from the request context or a chosen candidate).
pub async fn resolve_id(
    conn: &mut SqliteConnection,
    kind: EntityKind,
    id: &str,
) -> Result<Option<(String, String)>> {
    if id.is_empty() {
        return Ok(None);
    }
    let sql = match kind {
        EntityKind::Artist => "SELECT e.id,e.name,NULL AS subtitle FROM artists e WHERE e.id=?",
        EntityKind::Album => "SELECT e.id,e.name,NULL AS subtitle FROM albums e WHERE e.id=?",
        EntityKind::Track => {
            "SELECT e.id,e.title AS name,NULL AS subtitle FROM tracks e WHERE e.id=?"
        }
    };
    let rows: Vec<NameRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{}{sql}",
        crate::library::METADATA
    )))
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().next().map(|row| (row.id, row.name)))
}

async fn search_rows(
    conn: &mut SqliteConnection,
    kind: EntityKind,
    literal: &str,
) -> Result<Vec<NameRow>> {
    let pattern = crate::db::search_pattern(literal.trim())?;
    let column = match kind {
        EntityKind::Track => "e.title",
        _ => "e.name",
    };
    let sql = format!(
        "{} {} WHERE {column} LIKE ? ESCAPE '\\'",
        crate::library::METADATA,
        select_sql(kind)
    );
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(pattern)
        .fetch_all(&mut *conn)
        .await?)
}

async fn all_rows(conn: &mut SqliteConnection, kind: EntityKind) -> Result<Vec<NameRow>> {
    let sql = format!("{} {}", crate::library::METADATA, select_sql(kind));
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .fetch_all(&mut *conn)
        .await?)
}

fn select_sql(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Artist => {
            "SELECT e.id,e.name,CAST((SELECT COUNT(DISTINCT t.id) FROM track_artists ta JOIN tracks t ON t.id=ta.track_id WHERE ta.artist_id=e.id) AS TEXT)||' tracks' AS subtitle FROM artists e"
        }
        EntityKind::Album => {
            "SELECT e.id,e.name,COALESCE((SELECT group_concat(name, ', ') FROM (SELECT DISTINCT a.name FROM track_artists ta JOIN artists a ON a.id=ta.artist_id JOIN tracks t ON t.id=ta.track_id WHERE t.album_id=e.id ORDER BY a.name COLLATE NOCASE)),'') AS subtitle FROM albums e"
        }
        EntityKind::Track => {
            "SELECT e.id,e.title AS name,COALESCE((SELECT group_concat(name, ', ') FROM (SELECT a.name FROM track_artists ta JOIN artists a ON a.id=ta.artist_id WHERE ta.track_id=e.id ORDER BY ta.ordinal)),'')||CASE WHEN al.name IS NULL THEN '' ELSE ' · '||al.name END AS subtitle FROM tracks e LEFT JOIN albums al ON al.id=e.album_id"
        }
    }
}

fn cloned_row(row: &NameRow) -> NameRow {
    NameRow {
        id: row.id.clone(),
        name: row.name.clone(),
        subtitle: row.subtitle.clone(),
    }
}

/// Unicode-safe, dependency-free fold used only for matching; stored names and
/// the values shown to the user are never rewritten.
pub fn fold(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for character in input.chars() {
        if ('\u{0300}'..='\u{036f}').contains(&character) {
            continue;
        }
        for lower in character.to_lowercase() {
            match lower {
                'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => out.push('a'),
                'ç' | 'ć' | 'č' => out.push('c'),
                'ď' | 'đ' => out.push('d'),
                'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' => out.push('e'),
                'ğ' => out.push('g'),
                'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' => out.push('i'),
                'ł' => out.push('l'),
                'ñ' | 'ń' => out.push('n'),
                'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => out.push('o'),
                'ř' => out.push('r'),
                'ś' | 'š' => out.push('s'),
                'ť' => out.push('t'),
                'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => out.push('u'),
                'ý' | 'ÿ' => out.push('y'),
                'ź' | 'ż' => out.push('z'),
                'æ' => out.push_str("ae"),
                'œ' => out.push_str("oe"),
                'ß' => out.push_str("ss"),
                '’' | '‘' | '`' => out.push('\''),
                '–' | '—' => out.push('-'),
                other => out.push(other),
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::fold;

    #[test]
    fn fold_is_case_and_accent_insensitive_without_dropping_symbols() {
        assert_eq!(fold("Björk"), fold("BJORK"));
        assert_eq!(fold("Beyoncé"), "beyonce");
        assert_eq!(fold("AC/DC"), "ac/dc");
        assert_eq!(fold("Guns N’ Roses"), "guns n' roses");
        assert_eq!(fold("坂本龍一"), "坂本龍一");
        assert_eq!(fold("  Sigur   Rós "), "sigur ros");
        assert_eq!(fold("Sgt. Pepper's"), "sgt. pepper's");
        assert_eq!(fold("Straße"), "strasse");
    }
}

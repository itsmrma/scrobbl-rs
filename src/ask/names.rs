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
    resolve_in(conn, kind, literal, None).await
}

/// Like [`resolve`], restricted to the tracks or albums of one artist, or the
/// tracks of one album. Several editions of the same song by the same artists
/// are one answer: the first edition is returned and execution counts all of
/// them through [`track_editions`].
pub async fn resolve_in(
    conn: &mut SqliteConnection,
    kind: EntityKind,
    literal: &str,
    scope: Option<(EntityKind, &str)>,
) -> Result<Resolution> {
    let needle = fold(literal);
    if needle.is_empty() {
        return Ok(Resolution::NotFound);
    }
    let allowed = match scope {
        Some(scope) => Some(scope_members(conn, kind, scope).await?),
        None => None,
    };
    let in_scope = |row: &NameRow| allowed.as_ref().is_none_or(|ids| ids.contains(&row.id));
    let mut rows = search_rows(conn, kind, literal).await?;
    rows.retain(|row| in_scope(row));
    if rows.is_empty() {
        // SQLite LIKE folds ASCII case only. Fall back to a Rust fold so
        // "Bjork" still finds "Björk" without a new dependency.
        rows = all_rows(conn, kind).await?;
        rows.retain(|row| in_scope(row) && fold(&row.name).contains(&needle));
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
    if rows.len() > 1 && kind == EntityKind::Track && same_song(&rows) {
        rows.truncate(1);
    }
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

/// Tracks whose folded title and artist credit are identical are editions of
/// one song (single, album, deluxe). The album part of the subtitle differs.
fn same_song(rows: &[NameRow]) -> bool {
    let key = |row: &NameRow| {
        let artists = row
            .subtitle
            .as_deref()
            .unwrap_or_default()
            .split(" · ")
            .next()
            .unwrap_or_default()
            .to_owned();
        (fold(&row.name), fold(&artists))
    };
    let first = key(&rows[0]);
    !first.1.is_empty() && rows.iter().all(|row| key(row) == first)
}

/// Every track that is an edition of `track_id`: same folded title and the
/// same set of credited artists. Always contains `track_id` itself.
pub async fn track_editions(conn: &mut SqliteConnection, track_id: &str) -> Result<Vec<String>> {
    #[derive(FromRow)]
    struct Edition {
        id: String,
        title: String,
        artists: Option<String>,
    }
    let sql = format!(
        "{} SELECT e.id,e.title,(SELECT group_concat(artist_id, ',') FROM (SELECT ta.artist_id FROM track_artists ta WHERE ta.track_id=e.id ORDER BY ta.artist_id)) AS artists FROM tracks e WHERE e.id=? OR e.id IN (SELECT ta.track_id FROM track_artists ta WHERE ta.artist_id IN (SELECT artist_id FROM track_artists WHERE track_id=?))",
        crate::library::METADATA
    );
    let rows: Vec<Edition> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(track_id)
        .bind(track_id)
        .fetch_all(&mut *conn)
        .await?;
    let Some(target) = rows.iter().find(|row| row.id == track_id) else {
        return Ok(vec![track_id.to_owned()]);
    };
    let (title, artists) = (fold(&target.title), target.artists.clone());
    let mut ids: Vec<String> = rows
        .iter()
        .filter(|row| row.id == track_id || (fold(&row.title) == title && row.artists == artists))
        .map(|row| row.id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// IDs of the tracks/albums inside an artist or album scope.
async fn scope_members(
    conn: &mut SqliteConnection,
    kind: EntityKind,
    (scope_kind, scope_id): (EntityKind, &str),
) -> Result<std::collections::BTreeSet<String>> {
    let sql = match (kind, scope_kind) {
        (EntityKind::Track, EntityKind::Artist) => {
            "SELECT DISTINCT ta.track_id FROM track_artists ta WHERE ta.artist_id IN (SELECT entity_id FROM name_group_members WHERE kind='artists' AND group_id=(SELECT COALESCE((SELECT group_id FROM name_group_members WHERE kind='artists' AND entity_id=?1),?1)) UNION SELECT ?1)"
        }
        (EntityKind::Album, EntityKind::Artist) => {
            "SELECT DISTINCT COALESCE(m.group_id,t.album_id) FROM tracks t JOIN track_artists ta ON ta.track_id=t.id LEFT JOIN name_group_members m ON m.kind='albums' AND m.entity_id=t.album_id WHERE t.album_id IS NOT NULL AND ta.artist_id IN (SELECT entity_id FROM name_group_members WHERE kind='artists' AND group_id=(SELECT COALESCE((SELECT group_id FROM name_group_members WHERE kind='artists' AND entity_id=?1),?1)) UNION SELECT ?1)"
        }
        (EntityKind::Track, EntityKind::Album) => {
            "SELECT t.id FROM tracks t WHERE t.album_id IN (SELECT entity_id FROM name_group_members WHERE kind='albums' AND group_id=(SELECT COALESCE((SELECT group_id FROM name_group_members WHERE kind='albums' AND entity_id=?1),?1)) UNION SELECT ?1)"
        }
        _ => return Ok(Default::default()),
    };
    let ids: Vec<String> = sqlx::query_scalar(sql)
        .bind(scope_id)
        .fetch_all(&mut *conn)
        .await?;
    Ok(ids.into_iter().collect())
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

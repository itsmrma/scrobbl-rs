//! Parser seam between the Ask API and the question interpreter.
//!
//! The real local model adapter arrives in #178. It must implement the same
//! object-safe `QueryParser` trait and return only DSL, never SQL, rows or IDs.
//! CI and the current app use `StubParser`, a deterministic phrase map.
use crate::{
    ask::dsl::{Entity, EntityKind, Metric, Period, Query, SortBy, SortOrder},
    error::Result,
};
use std::{future::Future, pin::Pin};

pub type ParseFuture<'a> = Pin<Box<dyn Future<Output = Result<ParsedQuestion>> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedQuestion {
    Query(Query),
    Unsupported { reason: String },
}

pub trait QueryParser: Send + Sync {
    fn parse<'a>(&'a self, question: &'a str) -> ParseFuture<'a>;
}

/// Deterministic English/Italian phrase map used by CI and development.
pub struct StubParser;

impl QueryParser for StubParser {
    fn parse<'a>(&'a self, question: &'a str) -> ParseFuture<'a> {
        Box::pin(async move { Ok(stub_parse(question)) })
    }
}

/// Stand-in until the compiled runtime and a downloaded model are available.
/// The handler also checks `runtime_available`, so this is only a safety net.
pub struct UnavailableParser;

impl QueryParser for UnavailableParser {
    fn parse<'a>(&'a self, _question: &'a str) -> ParseFuture<'a> {
        Box::pin(async move {
            Ok(ParsedQuestion::Unsupported {
                reason: "The AI runtime is not available in this build.".into(),
            })
        })
    }
}

pub fn unsupported_question_message() -> String {
    "I can answer questions about play counts, listening time, distinct tracks, artists and albums, first plays, last plays, listening streaks, milestones and grouped top lists (metrics: plays, time_listened, distinct_tracks, distinct_artists, distinct_albums, first_play, last_play, streak, milestones). Try “How many times did I listen to Björk?” or “How many hours did I listen?”".into()
}

pub fn stub_parse(question: &str) -> ParsedQuestion {
    let question = normalise(question);
    if question.is_empty() {
        return ParsedQuestion::Unsupported {
            reason: unsupported_question_message(),
        };
    }

    if contains_any(
        &question,
        &["how many hours", "how long", "quante ore", "quanto tempo"],
    ) {
        return ParsedQuestion::Query(default_query(Metric::TimeListened));
    }
    if contains_any(
        &question,
        &[
            "distinct artist",
            "different artist",
            "distinct album",
            "distinct track",
            "quanti artisti",
            "quanti album",
            "quanti brani diversi",
        ],
    ) {
        let metric = if contains_any(&question, &["album"]) {
            Metric::DistinctAlbums
        } else if contains_any(&question, &["track", "brani"]) {
            Metric::DistinctTracks
        } else {
            Metric::DistinctArtists
        };
        return ParsedQuestion::Query(default_query(metric));
    }
    if contains_any(&question, &["streak", "serie di giorni"]) {
        return ParsedQuestion::Query(default_query(Metric::Streak));
    }
    if contains_any(&question, &["milestone", "traguardo"]) {
        return ParsedQuestion::Query(default_query(Metric::Milestones));
    }

    // First/last listening. Entity literals stay untouched apart from the
    // transport-level normalisation above; resolution happens in Rust.
    for (prefix, metric) in [
        ("when did i first listen to ", Metric::FirstPlay),
        ("when did i first play ", Metric::FirstPlay),
        ("when did i last listen to ", Metric::LastPlay),
        ("when did i last play ", Metric::LastPlay),
        ("when did i start listening to ", Metric::FirstPlay),
    ] {
        if let Some(literal) = question.strip_prefix(prefix) {
            return entity_query(metric, literal);
        }
    }
    for (prefix, suffix, metric) in [
        (
            "quando ho ascoltato ",
            " per la prima volta",
            Metric::FirstPlay,
        ),
        ("quando ho ascoltato ", " l'ultima volta", Metric::LastPlay),
        ("quando ho iniziato ad ascoltare ", "", Metric::FirstPlay),
    ] {
        if let Some(rest) = question.strip_prefix(prefix) {
            if !suffix.is_empty() && !rest.ends_with(suffix) {
                continue;
            }
            let literal = rest.strip_suffix(suffix).unwrap_or(rest);
            return entity_query(metric, literal);
        }
    }

    for prefix in [
        "how many times did i listen to ",
        "how many times have i listened to ",
        "how many times did i play ",
        "how many plays for ",
        "how many plays of ",
        "how many listens for ",
    ] {
        if let Some(literal) = question.strip_prefix(prefix) {
            return entity_query(Metric::Plays, literal);
        }
    }
    for prefix in [
        "quante volte ho ascoltato ",
        "quante volte ho riprodotto ",
        "quanti ascolti per ",
        "quanti ascolti di ",
    ] {
        if let Some(literal) = question.strip_prefix(prefix) {
            return entity_query(Metric::Plays, literal);
        }
    }

    if let Some(period) = period_only_question(&question) {
        return ParsedQuestion::Query(period);
    }

    // A question about a named entity with an unrecognised metric is still a
    // refusal, not an empty answer.
    ParsedQuestion::Unsupported {
        reason: unsupported_question_message(),
    }
}

fn entity_query(metric: Metric, literal: &str) -> ParsedQuestion {
    match entity_from(literal) {
        Some(entity) => {
            let mut query = default_query(metric);
            query.entity = Some(entity);
            ParsedQuestion::Query(query)
        }
        None => ParsedQuestion::Unsupported {
            reason: "Tell me which artist, album or track you mean.".into(),
        },
    }
}

fn entity_from(literal: &str) -> Option<Entity> {
    let mut rest = literal.trim();
    let mut kind = EntityKind::Artist;
    for (prefix, candidate) in [
        ("the artist ", EntityKind::Artist),
        ("artist ", EntityKind::Artist),
        ("the album ", EntityKind::Album),
        ("album ", EntityKind::Album),
        ("the track ", EntityKind::Track),
        ("track ", EntityKind::Track),
        ("the song ", EntityKind::Track),
        ("song ", EntityKind::Track),
        ("l'artista ", EntityKind::Artist),
        ("l'album ", EntityKind::Album),
        ("il brano ", EntityKind::Track),
        ("la canzone ", EntityKind::Track),
    ] {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            kind = candidate;
            rest = stripped;
            break;
        }
    }
    let name = rest.trim();
    (!name.is_empty()).then(|| Entity {
        kind,
        name: name.to_owned(),
        id: None,
    })
}

fn period_only_question(question: &str) -> Option<Query> {
    for prefix in [
        "how many plays",
        "how many listens",
        "how many times",
        "quanti ascolti",
        "quante volte",
    ] {
        let Some(suffix) = question.strip_prefix(prefix) else {
            continue;
        };
        let suffix = suffix.trim();
        let mut query = default_query(Metric::Plays);
        if suffix.is_empty() {
            return Some(query);
        }
        if let Some(rest) = suffix.strip_prefix("in ")
            && let Ok(year) = rest.parse::<i32>()
        {
            query.year = Some(year);
            query.period = Period::AllTime;
            return Some(query);
        }
        if let Some(rest) = suffix.strip_prefix("nel ")
            && let Ok(year) = rest.parse::<i32>()
        {
            query.year = Some(year);
            query.period = Period::AllTime;
            return Some(query);
        }
        let period = match suffix {
            "today" | "oggi" => Some(Period::Today),
            "yesterday" | "ieri" => Some(Period::Yesterday),
            "this week" | "questa settimana" => Some(Period::ThisWeek),
            "last week" | "la settimana scorsa" | "settimana scorsa" => Some(Period::LastWeek),
            "this month" | "questo mese" => Some(Period::ThisMonth),
            "last month" | "il mese scorso" | "mese scorso" => Some(Period::LastMonth),
            "this year" | "quest'anno" => Some(Period::ThisYear),
            "last year" | "l'anno scorso" | "anno scorso" => Some(Period::LastYear),
            _ => None,
        }?;
        query.period = period;
        return Some(query);
    }
    None
}

fn default_query(metric: Metric) -> Query {
    Query {
        metric,
        entity: None,
        period: Period::AllTime,
        year: None,
        month: None,
        weekday: None,
        hour: None,
        source: None,
        group_by: None,
        sort_by: SortBy::Metric,
        order: SortOrder::Desc,
        limit: None,
    }
}

fn contains_any(question: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| question.contains(needle))
}

fn normalise(question: &str) -> String {
    question
        .trim()
        .trim_end_matches(['?', '.', '!', ' '])
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_english_and_italian_counts() {
        let ParsedQuestion::Query(query) = stub_parse("How many times did I listen to Björk?")
        else {
            panic!("expected a query");
        };
        assert_eq!(query.metric, Metric::Plays);
        assert_eq!(
            query.entity,
            Some(Entity {
                kind: EntityKind::Artist,
                name: "björk".into(),
                id: None,
            })
        );

        let ParsedQuestion::Query(query) = stub_parse("Quante volte ho ascoltato Björk?") else {
            panic!("expected a query");
        };
        assert_eq!(query.metric, Metric::Plays);
    }

    #[test]
    fn parses_first_last_and_periods() {
        let ParsedQuestion::Query(query) = stub_parse("When did I first listen to Radiohead?")
        else {
            panic!("expected a query");
        };
        assert_eq!(query.metric, Metric::FirstPlay);

        let ParsedQuestion::Query(query) = stub_parse("quando ho ascoltato Björk l'ultima volta")
        else {
            panic!("expected a query");
        };
        assert_eq!(query.metric, Metric::LastPlay);

        let ParsedQuestion::Query(query) = stub_parse("How many plays in 2024?") else {
            panic!("expected a query");
        };
        assert_eq!(query.year, Some(2024));
        assert_eq!(query.period, Period::AllTime);

        let ParsedQuestion::Query(query) = stub_parse("quanti ascolti quest'anno") else {
            panic!("expected a query");
        };
        assert_eq!(query.period, Period::ThisYear);
    }

    #[test]
    fn refuses_out_of_scope_questions_explicitly() {
        let ParsedQuestion::Unsupported { reason } = stub_parse("What is the weather?") else {
            panic!("expected a refusal");
        };
        assert!(reason.contains("play counts"));
        assert!(reason.contains("first plays"));
        assert!(reason.contains("last plays"));
    }

    #[test]
    fn the_stub_is_deterministic_and_has_no_free_form_numbers() {
        let first = stub_parse("How many times did I listen to Björk?");
        let second = stub_parse("  how many times did i listen to björk  ");
        assert_eq!(first, second);
        let ParsedQuestion::Query(query) = first else {
            panic!("expected a query");
        };
        assert!(query.limit.is_none());
    }
}

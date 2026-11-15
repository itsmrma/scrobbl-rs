//! The model-facing Ask output and the JSON Schema that constrains it.
//!
//! The local model never emits the whole DSL: dates, years, months, hours,
//! sources and numeric limits are filled by Rust or selected through enums. The
//! schema is generated from the same enum lists the deserializer uses, and a
//! unit test keeps `schema/ask.schema.json` identical to that generation so the
//! grammar cannot drift from the Rust types.
use crate::{
    ask::{
        dsl::{
            Entity, EntityKind, GroupBy, Limit, Metric, Period, Query, SortBy, SortOrder, Weekday,
        },
        parser::{ParsedQuestion, unsupported_question_message},
    },
    error::Result,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Declare a string enum together with the exact list of accepted values.
///
/// Serialization, deserialization and the generated schema all use `VALUES`,
/// so adding or renaming a variant cannot silently diverge from the grammar.
macro_rules! model_enum {
    ($(#[$meta:meta])* $vis:vis $name:ident { $($variant:ident = $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        $vis enum $name {
            $($variant),+
        }

        impl $name {
            pub const VALUES: &'static [&'static str] = &[$($value),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            pub fn parse_value(value: &str) -> Option<Self> {
                match value {
                    $($value => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> std::result::Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Self::parse_value(&value).ok_or_else(|| {
                    serde::de::Error::unknown_variant(&value, Self::VALUES)
                })
            }
        }
    };
}

model_enum! {
    /// Metrics the model may choose. `unsupported` is an explicit refusal, not
    /// a metric the executor understands.
    pub ModelMetric {
        Plays = "plays",
        TimeListened = "time_listened",
        DistinctTracks = "distinct_tracks",
        DistinctArtists = "distinct_artists",
        DistinctAlbums = "distinct_albums",
        FirstPlay = "first_play",
        LastPlay = "last_play",
        Streak = "streak",
        Milestones = "milestones",
        Rank = "rank",
        Share = "share",
        ListeningDays = "listening_days",
        Unsupported = "unsupported",
    }
}

model_enum! {
    pub ModelTarget {
        Artist = "artist",
        Album = "album",
        Track = "track",
        None = "none",
    }
}

model_enum! {
    pub ModelPeriod {
        AllTime = "all_time",
        Today = "today",
        Yesterday = "yesterday",
        ThisWeek = "this_week",
        LastWeek = "last_week",
        ThisMonth = "this_month",
        LastMonth = "last_month",
        ThisYear = "this_year",
        LastYear = "last_year",
        Last7Days = "last_7_days",
        Last30Days = "last_30_days",
        Last365Days = "last_365_days",
    }
}

model_enum! {
    pub ModelMonth {
        None = "none",
        January = "january",
        February = "february",
        March = "march",
        April = "april",
        May = "may",
        June = "june",
        July = "july",
        August = "august",
        September = "september",
        October = "october",
        November = "november",
        December = "december",
    }
}

model_enum! {
    pub ModelWeekday {
        Any = "any",
        Monday = "monday",
        Tuesday = "tuesday",
        Wednesday = "wednesday",
        Thursday = "thursday",
        Friday = "friday",
        Saturday = "saturday",
        Sunday = "sunday",
    }
}

model_enum! {
    pub ModelGroupBy {
        None = "none",
        Day = "day",
        Week = "week",
        Month = "month",
        Year = "year",
        Hour = "hour",
        Weekday = "weekday",
        Artist = "artist",
        Album = "album",
        Track = "track",
    }
}

model_enum! {
    pub ModelOrder {
        Asc = "asc",
        Desc = "desc",
    }
}

model_enum! {
    pub ModelLimit {
        None = "none",
        One = "one",
        Five = "five",
        Ten = "ten",
        TwentyFive = "twenty_five",
        Fifty = "fifty",
        Hundred = "hundred",
    }
}

/// Exactly what the grammar allows the model to emit. Every field is required
/// and every enum field only accepts the values declared above.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelOutput {
    /// v2 fields default so recorded v1 outputs still parse; the grammar
    /// always emits them.
    #[serde(default)]
    pub artist: String,
    pub group_by: ModelGroupBy,
    pub limit: ModelLimit,
    pub metric: ModelMetric,
    #[serde(default = "no_month")]
    pub month: ModelMonth,
    pub name: String,
    pub order: ModelOrder,
    pub period: ModelPeriod,
    pub target: ModelTarget,
    pub weekday: ModelWeekday,
    #[serde(default)]
    pub year: i64,
}

fn no_month() -> ModelMonth {
    ModelMonth::None
}

/// Alphabetical, matching serde_json's map order used by the grammar.
const FIELD_ORDER: [&str; 11] = [
    "artist", "group_by", "limit", "metric", "month", "name", "order", "period", "target",
    "weekday", "year",
];

fn enum_schema(values: &[&str], description: &str) -> Value {
    json!({
        "enum": values,
        "description": description,
    })
}

/// JSON Schema subset consumed by `grammar::to_gbnf`. All properties are
/// required so the generated grammar has a finite key order; optional filters
/// use explicit `none`/`any` enum values instead of missing keys.
pub fn schema_json() -> Value {
    let mut properties = Map::new();
    properties.insert(
        "artist".into(),
        json!({
            "type": "string",
            "description": "Literal artist name that narrows a named track or album (\"Show by Ado\"), or whose tracks/albums are counted or grouped (\"my top songs by Ado\"); empty otherwise.",
        }),
    );
    // Insertion order matches `FIELD_ORDER`; serde_json's default map sorts
    // alphabetically, which is the same order, so the grammar stays stable.
    properties.insert(
        "group_by".into(),
        enum_schema(
            ModelGroupBy::VALUES,
            "Grouping dimension, or none when the answer is a single value.",
        ),
    );
    properties.insert(
        "limit".into(),
        enum_schema(
            ModelLimit::VALUES,
            "Result limit, or none when the question does not ask for a top-N.",
        ),
    );
    properties.insert(
        "metric".into(),
        enum_schema(
            ModelMetric::VALUES,
            "Requested metric; unsupported means the question is outside the schema.",
        ),
    );
    properties.insert(
        "month".into(),
        enum_schema(
            ModelMonth::VALUES,
            "Calendar month named by the question, or none.",
        ),
    );
    properties.insert(
        "name".into(),
        json!({
            "type": "string",
            "description": "Literal artist, album or track name copied from the question; empty when target is none.",
        }),
    );
    properties.insert(
        "order".into(),
        enum_schema(ModelOrder::VALUES, "Sort direction."),
    );
    properties.insert(
        "period".into(),
        enum_schema(ModelPeriod::VALUES, "Listening period."),
    );
    properties.insert(
        "target".into(),
        enum_schema(
            ModelTarget::VALUES,
            "Entity kind named by the question, or none.",
        ),
    );
    properties.insert(
        "weekday".into(),
        enum_schema(
            ModelWeekday::VALUES,
            "Weekday filter, or any when no weekday is mentioned.",
        ),
    );
    properties.insert(
        "year".into(),
        json!({
            "type": "integer",
            "description": "Four-digit calendar year named by the question, or 0 when no year is named.",
        }),
    );

    json!({
        "$id": "https://scrobbl.rs/schemas/ask.v2.json",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Ask model output v2",
        "description": "Model-facing Ask output. Rust maps it to the shared DSL, validates it and computes every number.",
        "type": "object",
        "additionalProperties": false,
        "required": FIELD_ORDER,
        "properties": properties,
    })
}

pub fn schema_text() -> String {
    let mut text = serde_json::to_string_pretty(&schema_json())
        .expect("the Ask schema is a serializable JSON value");
    text.push('\n');
    text
}

fn metric(self_metric: ModelMetric) -> Option<Metric> {
    Some(match self_metric {
        ModelMetric::Plays => Metric::Plays,
        ModelMetric::TimeListened => Metric::TimeListened,
        ModelMetric::DistinctTracks => Metric::DistinctTracks,
        ModelMetric::DistinctArtists => Metric::DistinctArtists,
        ModelMetric::DistinctAlbums => Metric::DistinctAlbums,
        ModelMetric::FirstPlay => Metric::FirstPlay,
        ModelMetric::LastPlay => Metric::LastPlay,
        ModelMetric::Streak => Metric::Streak,
        ModelMetric::Milestones => Metric::Milestones,
        ModelMetric::Rank => Metric::Rank,
        ModelMetric::Share => Metric::Share,
        ModelMetric::ListeningDays => Metric::ListeningDays,
        ModelMetric::Unsupported => return None,
    })
}

fn entity_kind(target: ModelTarget) -> Option<EntityKind> {
    Some(match target {
        ModelTarget::Artist => EntityKind::Artist,
        ModelTarget::Album => EntityKind::Album,
        ModelTarget::Track => EntityKind::Track,
        ModelTarget::None => return None,
    })
}

fn period(period: ModelPeriod) -> Period {
    match period {
        ModelPeriod::AllTime => Period::AllTime,
        ModelPeriod::Today => Period::Today,
        ModelPeriod::Yesterday => Period::Yesterday,
        ModelPeriod::ThisWeek => Period::ThisWeek,
        ModelPeriod::LastWeek => Period::LastWeek,
        ModelPeriod::ThisMonth => Period::ThisMonth,
        ModelPeriod::LastMonth => Period::LastMonth,
        ModelPeriod::ThisYear => Period::ThisYear,
        ModelPeriod::LastYear => Period::LastYear,
        ModelPeriod::Last7Days => Period::Last7Days,
        ModelPeriod::Last30Days => Period::Last30Days,
        ModelPeriod::Last365Days => Period::Last365Days,
    }
}

fn month(month: ModelMonth) -> Option<u8> {
    Some(match month {
        ModelMonth::None => return None,
        ModelMonth::January => 1,
        ModelMonth::February => 2,
        ModelMonth::March => 3,
        ModelMonth::April => 4,
        ModelMonth::May => 5,
        ModelMonth::June => 6,
        ModelMonth::July => 7,
        ModelMonth::August => 8,
        ModelMonth::September => 9,
        ModelMonth::October => 10,
        ModelMonth::November => 11,
        ModelMonth::December => 12,
    })
}

fn weekday(weekday: ModelWeekday) -> Option<Weekday> {
    Some(match weekday {
        ModelWeekday::Any => return None,
        ModelWeekday::Monday => Weekday::Monday,
        ModelWeekday::Tuesday => Weekday::Tuesday,
        ModelWeekday::Wednesday => Weekday::Wednesday,
        ModelWeekday::Thursday => Weekday::Thursday,
        ModelWeekday::Friday => Weekday::Friday,
        ModelWeekday::Saturday => Weekday::Saturday,
        ModelWeekday::Sunday => Weekday::Sunday,
    })
}

fn group_by(group_by: ModelGroupBy) -> Option<GroupBy> {
    Some(match group_by {
        ModelGroupBy::None => return None,
        ModelGroupBy::Day => GroupBy::Day,
        ModelGroupBy::Week => GroupBy::Week,
        ModelGroupBy::Month => GroupBy::Month,
        ModelGroupBy::Year => GroupBy::Year,
        ModelGroupBy::Hour => GroupBy::Hour,
        ModelGroupBy::Weekday => GroupBy::Weekday,
        ModelGroupBy::Artist => GroupBy::Artist,
        ModelGroupBy::Album => GroupBy::Album,
        ModelGroupBy::Track => GroupBy::Track,
    })
}

fn limit(limit: ModelLimit) -> Option<Limit> {
    Some(match limit {
        ModelLimit::None => return None,
        ModelLimit::One => Limit::One,
        ModelLimit::Five => Limit::Five,
        ModelLimit::Ten => Limit::Ten,
        ModelLimit::TwentyFive => Limit::TwentyFive,
        ModelLimit::Fifty => Limit::Fifty,
        ModelLimit::Hundred => Limit::Hundred,
    })
}

fn contains_any(question: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| question.contains(needle))
}

/// The calendar dimension a "which day/month/…" question asks about.
fn temporal_dimension(question: &str) -> Option<ModelGroupBy> {
    // The weekday wording contains the day wording, so it is checked first.
    if contains_any(
        question,
        &[
            "giorno della settimana",
            "giorni della settimana",
            "day of the week",
            "days of the week",
            "which weekday",
            "what weekday",
        ],
    ) {
        Some(ModelGroupBy::Weekday)
    } else if contains_any(
        question,
        &[
            "che ora ",
            "che ore ",
            "quale ora",
            "quali ore",
            "a che ora",
            "what hour",
            "which hour",
            "what time of day",
            "which time of day",
        ],
    ) {
        Some(ModelGroupBy::Hour)
    } else if contains_any(
        question,
        &[
            "che giorno",
            "quale giorno",
            "quali giorni",
            "which day",
            "what day",
            "which days",
        ],
    ) {
        Some(ModelGroupBy::Day)
    } else if contains_any(
        question,
        &[
            "che settimana",
            "quale settimana",
            "quali settimane",
            "which week",
            "what week",
        ],
    ) {
        Some(ModelGroupBy::Week)
    } else if contains_any(
        question,
        &[
            "che mese",
            "quale mese",
            "quali mesi",
            "which month",
            "what month",
        ],
    ) {
        Some(ModelGroupBy::Month)
    } else if contains_any(
        question,
        &[
            "che anno",
            "quale anno",
            "quali anni",
            "which year",
            "what year",
        ],
    ) {
        Some(ModelGroupBy::Year)
    } else {
        None
    }
}

fn mentions_least(question: &str) -> bool {
    contains_any(question, &[" meno", "least", "fewest", "lowest", "minimo"])
}

fn mentions_most(question: &str) -> bool {
    contains_any(
        question,
        &[
            " più",
            "maggior",
            "massimo",
            "most",
            "highest",
            "more times",
            "the more",
            "biggest",
        ],
    )
}

fn invalid_output() -> ParsedQuestion {
    ParsedQuestion::Unsupported {
        reason: "The local model produced an interpretation that does not match the question format. Try rephrasing the question.".into(),
    }
}

impl ModelOutput {
    /// Repair what the small local model most often misses: a question that
    /// asks *which* day, month, hour, weekday, week or year is a grouped
    /// top-N, not one total. Only an ungrouped plays question is touched, and
    /// only when the wording is unambiguous, so a correct model answer is
    /// never overridden.
    pub fn apply_question_hints(&mut self, question: &str) {
        if self.metric != ModelMetric::Plays || self.group_by != ModelGroupBy::None {
            return;
        }
        let question = question.to_lowercase();
        let Some(group_by) = temporal_dimension(&question) else {
            return;
        };
        self.group_by = group_by;
        if self.limit == ModelLimit::None {
            if mentions_least(&question) {
                self.limit = ModelLimit::One;
                self.order = ModelOrder::Asc;
            } else if mentions_most(&question) {
                self.limit = ModelLimit::One;
                self.order = ModelOrder::Desc;
            }
        }
    }

    /// Map validated model output onto the shared DSL. Model output that is
    /// internally inconsistent becomes an explicit refusal instead of a
    /// guessed query.
    pub fn into_parsed(self) -> Result<ParsedQuestion> {
        let Some(metric) = metric(self.metric) else {
            return Ok(ParsedQuestion::Unsupported {
                reason: unsupported_question_message(),
            });
        };

        let artist = self.artist.trim();
        let artist_entity = (!artist.is_empty()).then(|| Entity {
            kind: EntityKind::Artist,
            name: artist.to_string(),
            id: None,
        });
        let (entity, scope) = match entity_kind(self.target) {
            None => {
                if !self.name.trim().is_empty() {
                    return Ok(invalid_output());
                }
                // "My top songs by Ado": the artist is the subject.
                (artist_entity, None)
            }
            Some(kind) => {
                let name = self.name.trim();
                if name.is_empty() {
                    // A contextual question may not repeat the entity name.
                    // Rust fills it from the request context before execution.
                    (
                        if kind == EntityKind::Artist {
                            artist_entity
                        } else {
                            None
                        },
                        None,
                    )
                } else if kind == EntityKind::Artist {
                    (
                        Some(Entity {
                            kind,
                            name: name.to_string(),
                            id: None,
                        }),
                        None,
                    )
                } else {
                    (
                        Some(Entity {
                            kind,
                            name: name.to_string(),
                            id: None,
                        }),
                        artist_entity,
                    )
                }
            }
        };
        let year = match self.year {
            0 => None,
            year @ 1900..=2200 => Some(year as i32),
            _ => return Ok(invalid_output()),
        };

        let group_by = group_by(self.group_by);
        // A grouped time series is read chronologically; a top-N ("which day
        // did I listen the most") is read by the metric, otherwise the newest
        // or oldest bucket would win. `sort_by` is deliberately not part of the
        // model schema.
        let limit = limit(self.limit);
        let sort_by = match group_by {
            Some(
                GroupBy::Day
                | GroupBy::Week
                | GroupBy::Month
                | GroupBy::Year
                | GroupBy::Hour
                | GroupBy::Weekday,
            ) if limit.is_none() => SortBy::Date,
            _ => SortBy::Metric,
        };

        let query = Query {
            metric,
            entity,
            scope,
            // A named year replaces the relative period.
            period: if year.is_some() {
                Period::AllTime
            } else {
                period(self.period)
            },
            year,
            month: month(self.month),
            weekday: weekday(self.weekday),
            hour: None,
            source: None,
            group_by,
            sort_by,
            order: match self.order {
                ModelOrder::Asc => SortOrder::Asc,
                ModelOrder::Desc => SortOrder::Desc,
            },
            limit,
        };
        query.validate()?;
        Ok(ParsedQuestion::Query(query))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHECKED_IN: &str = include_str!("schema/ask.schema.json");

    #[test]
    fn checked_in_schema_matches_the_generated_one() {
        let generated = schema_text();
        if std::env::var_os("UPDATE_ASK_SCHEMA").is_some() {
            std::fs::write(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/src/ask/schema/ask.schema.json"
                ),
                &generated,
            )
            .expect("write the Ask schema");
        }
        assert_eq!(
            CHECKED_IN, generated,
            "src/ask/schema/ask.schema.json is stale; run \
             $env:UPDATE_ASK_SCHEMA='1'; cargo test checked_in_schema_matches_the_generated_one"
        );
    }

    #[test]
    fn every_enum_value_is_in_the_schema() {
        let schema = schema_json();
        let properties = schema["properties"].as_object().unwrap();
        assert_eq!(
            schema["required"],
            json!(FIELD_ORDER),
            "every field must be required"
        );
        for (field, values) in [
            ("metric", ModelMetric::VALUES),
            ("target", ModelTarget::VALUES),
            ("period", ModelPeriod::VALUES),
            ("weekday", ModelWeekday::VALUES),
            ("month", ModelMonth::VALUES),
            ("group_by", ModelGroupBy::VALUES),
            ("order", ModelOrder::VALUES),
            ("limit", ModelLimit::VALUES),
        ] {
            assert_eq!(properties[field]["enum"], json!(values), "{field}");
        }
    }

    #[test]
    fn schema_round_trips_through_the_model_output_enum_lists() {
        for value in ModelMetric::VALUES {
            assert!(ModelMetric::parse_value(value).is_some(), "{value}");
        }
        assert!(ModelMetric::parse_value("top").is_none());
    }

    fn plays_output(group_by: ModelGroupBy) -> ModelOutput {
        ModelOutput {
            artist: String::new(),
            group_by,
            limit: ModelLimit::None,
            metric: ModelMetric::Plays,
            month: ModelMonth::None,
            name: "BTS".into(),
            order: ModelOrder::Desc,
            period: ModelPeriod::AllTime,
            target: ModelTarget::Artist,
            weekday: ModelWeekday::Any,
            year: 0,
        }
    }

    #[test]
    fn which_day_question_becomes_a_grouped_top_one() {
        for (question, expected) in [
            (
                "In che giorno ho ascoltato più volte i BTS?",
                ModelGroupBy::Day,
            ),
            ("Which day did I play BTS the most?", ModelGroupBy::Day),
            (
                "In quale giorno della settimana ascolto di più i BTS?",
                ModelGroupBy::Weekday,
            ),
            ("A che ora ascolto più BTS?", ModelGroupBy::Hour),
            (
                "What month did I listen to BTS the most?",
                ModelGroupBy::Month,
            ),
            ("In che anno ho ascoltato di più i BTS?", ModelGroupBy::Year),
        ] {
            let mut output = plays_output(ModelGroupBy::None);
            output.apply_question_hints(question);
            assert_eq!(output.group_by, expected, "{question}");
            assert_eq!(output.limit, ModelLimit::One, "{question}");
            assert_eq!(output.order, ModelOrder::Desc, "{question}");
        }
    }

    #[test]
    fn the_hint_is_a_top_n_ranked_by_the_metric() {
        let mut output = plays_output(ModelGroupBy::None);
        output.apply_question_hints("In che giorno ho ascoltato più volte i BTS?");
        let ParsedQuestion::Query(query) = output.into_parsed().unwrap() else {
            panic!("expected a query");
        };
        assert_eq!(query.group_by, Some(GroupBy::Day));
        assert_eq!(query.sort_by, SortBy::Metric);
        assert_eq!(query.order, SortOrder::Desc);
        assert_eq!(query.limit, Some(Limit::One));
        assert_eq!(query.entity.map(|entity| entity.name), Some("BTS".into()));
    }

    #[test]
    fn the_hint_never_overrides_a_grouped_or_unrelated_answer() {
        let mut grouped = plays_output(ModelGroupBy::Artist);
        grouped.apply_question_hints("In che giorno ho ascoltato più volte i BTS?");
        assert_eq!(grouped.group_by, ModelGroupBy::Artist);

        let mut total = plays_output(ModelGroupBy::None);
        total.apply_question_hints("Quante volte ho ascoltato i BTS?");
        assert_eq!(total.group_by, ModelGroupBy::None);
        assert_eq!(total.limit, ModelLimit::None);

        let mut refusal = plays_output(ModelGroupBy::None);
        refusal.metric = ModelMetric::Unsupported;
        refusal.apply_question_hints("Which day is it?");
        assert_eq!(refusal.group_by, ModelGroupBy::None);
    }
}

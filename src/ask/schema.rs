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
    pub group_by: ModelGroupBy,
    pub limit: ModelLimit,
    pub metric: ModelMetric,
    pub name: String,
    pub order: ModelOrder,
    pub period: ModelPeriod,
    pub target: ModelTarget,
    pub weekday: ModelWeekday,
}

const FIELD_ORDER: [&str; 8] = [
    "group_by", "limit", "metric", "name", "order", "period", "target", "weekday",
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

    json!({
        "$id": "https://scrobbl.rs/schemas/ask.v1.json",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Ask model output v1",
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
    }
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

fn invalid_output() -> ParsedQuestion {
    ParsedQuestion::Unsupported {
        reason: "The local model produced an interpretation that does not match the question format. Try rephrasing the question.".into(),
    }
}

impl ModelOutput {
    /// Map validated model output onto the shared DSL. Model output that is
    /// internally inconsistent becomes an explicit refusal instead of a
    /// guessed query.
    pub fn into_parsed(self) -> Result<ParsedQuestion> {
        let Some(metric) = metric(self.metric) else {
            return Ok(ParsedQuestion::Unsupported {
                reason: unsupported_question_message(),
            });
        };

        let entity = match entity_kind(self.target) {
            None => {
                if !self.name.trim().is_empty() {
                    return Ok(invalid_output());
                }
                None
            }
            Some(kind) => {
                let name = self.name.trim();
                if name.is_empty() {
                    // A contextual question may not repeat the entity name.
                    // Rust fills it from the request context before execution.
                    None
                } else {
                    Some(Entity {
                        kind,
                        name: name.to_string(),
                        id: None,
                    })
                }
            }
        };

        let group_by = group_by(self.group_by);
        // A grouped time series is read chronologically; a top-N is read by
        // the metric. `sort_by` is deliberately not part of the model schema.
        let sort_by = match group_by {
            Some(
                GroupBy::Day
                | GroupBy::Week
                | GroupBy::Month
                | GroupBy::Year
                | GroupBy::Hour
                | GroupBy::Weekday,
            ) => SortBy::Date,
            _ => SortBy::Metric,
        };

        let query = Query {
            metric,
            entity,
            period: period(self.period),
            year: None,
            month: None,
            weekday: weekday(self.weekday),
            hour: None,
            source: None,
            group_by,
            sort_by,
            order: match self.order {
                ModelOrder::Asc => SortOrder::Asc,
                ModelOrder::Desc => SortOrder::Desc,
            },
            limit: limit(self.limit),
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
}

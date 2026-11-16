//! Shared Ask query DSL. Rust validates and executes it; the model only emits it.
//!
//! Play counts, listening time, distinct tracks/artists/albums, first/last play,
//! streaks, milestones, ranking position, share of plays and listening days are
//! executable. Plays and the distinct metrics can be grouped, sorted and
//! limited; grouping any other metric is not implemented.
use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Query {
    pub metric: Metric,
    pub entity: Option<Entity>,
    /// Artist or album whose catalogue the entity name is looked up in, for
    /// example the artist in "Show by Ado". Used only to resolve the entity.
    #[serde(default)]
    pub scope: Option<Entity>,
    pub period: Period,
    /// Chip/API only; never emitted by the grammar.
    pub year: Option<i32>,
    /// 1..=12
    pub month: Option<u8>,
    pub weekday: Option<Weekday>,
    /// 0..=23, in the listen's assigned calendar timezone.
    pub hour: Option<u8>,
    pub source: Option<String>,
    /// Inclusive `YYYY-MM-DD` range in the assigned calendar. It replaces the
    /// period and the year when set.
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    pub group_by: Option<GroupBy>,
    pub sort_by: SortBy,
    pub order: SortOrder,
    pub limit: Option<Limit>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Plays,
    TimeListened,
    DistinctTracks,
    DistinctArtists,
    DistinctAlbums,
    FirstPlay,
    LastPlay,
    Streak,
    Milestones,
    /// Position of the entity among entities of its kind, by plays.
    Rank,
    /// Entity plays as a share of all plays in the same selection.
    Share,
    /// Distinct calendar days with at least one counted play.
    ListeningDays,
}

impl Metric {
    pub fn label(self) -> &'static str {
        match self {
            Self::Plays => "Plays",
            Self::TimeListened => "Time listened",
            Self::DistinctTracks => "Distinct tracks",
            Self::DistinctArtists => "Distinct artists",
            Self::DistinctAlbums => "Distinct albums",
            Self::FirstPlay => "First play",
            Self::LastPlay => "Last play",
            Self::Streak => "Listening streak",
            Self::Milestones => "Milestones",
            Self::Rank => "Ranking position",
            Self::Share => "Share of plays",
            Self::ListeningDays => "Listening days",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Artist,
    Album,
    Track,
}

impl EntityKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Artist => "Artist",
            Self::Album => "Album",
            Self::Track => "Track",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Entity {
    pub kind: EntityKind,
    /// Literal name from the question; resolved against the library by Rust.
    pub name: String,
    /// Set after resolution or by an explicit candidate/context choice.
    pub id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    AllTime,
    Today,
    Yesterday,
    ThisWeek,
    LastWeek,
    ThisMonth,
    LastMonth,
    ThisYear,
    LastYear,
    #[serde(rename = "last_7_days")]
    Last7Days,
    #[serde(rename = "last_30_days")]
    Last30Days,
    /// Rolling three months: "last 3 months", "ultimi tre mesi".
    #[serde(rename = "last_90_days")]
    Last90Days,
    /// Rolling six months: "last 6 months", "ultimi sei mesi".
    #[serde(rename = "last_180_days")]
    Last180Days,
    #[serde(rename = "last_365_days")]
    Last365Days,
}

impl Period {
    pub fn label(self) -> &'static str {
        match self {
            Self::AllTime => "All time",
            Self::Today => "Today",
            Self::Yesterday => "Yesterday",
            Self::ThisWeek => "This week",
            Self::LastWeek => "Last week",
            Self::ThisMonth => "This month",
            Self::LastMonth => "Last month",
            Self::ThisYear => "This year",
            Self::LastYear => "Last year",
            Self::Last7Days => "Last 7 days",
            Self::Last30Days => "Last 30 days",
            Self::Last90Days => "Last 90 days",
            Self::Last180Days => "Last 180 days",
            Self::Last365Days => "Last 365 days",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    pub fn label(self) -> &'static str {
        match self {
            Self::Monday => "Monday",
            Self::Tuesday => "Tuesday",
            Self::Wednesday => "Wednesday",
            Self::Thursday => "Thursday",
            Self::Friday => "Friday",
            Self::Saturday => "Saturday",
            Self::Sunday => "Sunday",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    Day,
    Week,
    Month,
    Year,
    Hour,
    Weekday,
    Artist,
    Album,
    Track,
}

impl GroupBy {
    pub fn label(self) -> &'static str {
        match self {
            Self::Day => "Day",
            Self::Week => "Week",
            Self::Month => "Month",
            Self::Year => "Year",
            Self::Hour => "Hour",
            Self::Weekday => "Weekday",
            Self::Artist => "Artist",
            Self::Album => "Album",
            Self::Track => "Track",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    Metric,
    Date,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    One,
    Five,
    Ten,
    TwentyFive,
    Fifty,
    Hundred,
}

impl Limit {
    pub const MAX: i64 = 100;

    pub fn value(self) -> i64 {
        match self {
            Self::One => 1,
            Self::Five => 5,
            Self::Ten => 10,
            Self::TwentyFive => 25,
            Self::Fifty => 50,
            Self::Hundred => 100,
        }
    }
}

/// One editable interpretation chip. The key names the `Query` field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Chip {
    pub key: String,
    pub label: String,
    pub value: String,
}

impl Query {
    /// Reject DSL that is outside the declared bounds. Metric availability is a
    /// separate execution concern: valid-but-unimplemented metrics are not errors.
    pub fn validate(&self) -> Result<()> {
        if let Some(entity) = &self.entity {
            let name = entity.name.trim();
            if name.is_empty() && entity.id.as_deref().is_none_or(str::is_empty) {
                return Err(AppError::BadRequest(
                    "Choose an artist, album or track name.".into(),
                ));
            }
            if name.chars().count() > 300 {
                return Err(AppError::BadRequest("The entity name is too long.".into()));
            }
            if entity.id.as_deref().is_some_and(str::is_empty) {
                return Err(AppError::BadRequest("The entity id is empty.".into()));
            }
        }
        if let Some(scope) = &self.scope {
            if scope.kind == EntityKind::Track {
                return Err(AppError::BadRequest(
                    "Only an artist or an album can narrow a name.".into(),
                ));
            }
            if scope.name.trim().is_empty() && scope.id.as_deref().is_none_or(str::is_empty) {
                return Err(AppError::BadRequest("The scope name is empty.".into()));
            }
            if scope.name.chars().count() > 300 {
                return Err(AppError::BadRequest("The scope name is too long.".into()));
            }
        }
        if let Some(year) = self.year
            && !(1..=9999).contains(&year)
        {
            return Err(AppError::BadRequest(
                "The year must be between 1 and 9999.".into(),
            ));
        }
        if self.month.is_some_and(|month| !(1..=12).contains(&month)) {
            return Err(AppError::BadRequest(
                "The month must be between 1 and 12.".into(),
            ));
        }
        if self.hour.is_some_and(|hour| hour > 23) {
            return Err(AppError::BadRequest(
                "The hour must be between 0 and 23.".into(),
            ));
        }
        if let Some(source) = &self.source {
            if source.trim().is_empty() {
                return Err(AppError::BadRequest("The source is empty.".into()));
            }
            if source.chars().count() > 200 {
                return Err(AppError::BadRequest("The source is too long.".into()));
            }
        }
        let parse = |value: &Option<String>, name: &str| -> Result<Option<chrono::NaiveDate>> {
            value
                .as_deref()
                .map(|value| {
                    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
                        AppError::BadRequest(format!("Use the {name} date as YYYY-MM-DD."))
                    })
                })
                .transpose()
        };
        if let (Some(from), Some(to)) = (parse(&self.from, "from")?, parse(&self.to, "to")?)
            && from > to
        {
            return Err(AppError::BadRequest(
                "The from date must not be after the to date.".into(),
            ));
        }
        Ok(())
    }

    /// Deterministic interpretation chips, in a stable display order.
    pub fn chips(&self) -> Vec<Chip> {
        let mut chips = vec![chip("metric", "Metric", self.metric.label())];
        if let Some(entity) = &self.entity {
            chips.push(chip(
                "entity",
                entity.kind.label(),
                entity.name.trim().to_owned(),
            ));
        }
        if let Some(scope) = &self.scope {
            chips.push(chip(
                "scope",
                match scope.kind {
                    EntityKind::Album => "In album",
                    _ => "By artist",
                },
                scope.name.trim().to_owned(),
            ));
        }
        chips.push(chip("period", "Period", self.period.label()));
        if let Some(year) = self.year {
            chips.push(chip("year", "Year", year.to_string()));
        }
        if let Some(month) = self.month {
            chips.push(chip("month", "Month", month.to_string()));
        }
        if let Some(from) = &self.from {
            chips.push(chip("from", "From", from.clone()));
        }
        if let Some(to) = &self.to {
            chips.push(chip("to", "To", to.clone()));
        }
        if let Some(weekday) = self.weekday {
            chips.push(chip("weekday", "Weekday", weekday.label()));
        }
        if let Some(hour) = self.hour {
            chips.push(chip("hour", "Hour", format!("{hour:02}:00")));
        }
        if let Some(source) = &self.source {
            chips.push(chip("source", "Source", source.clone()));
        }
        if let Some(group_by) = self.group_by {
            chips.push(chip("group_by", "Group by", group_by.label()));
        }
        chips.push(chip(
            "sort_by",
            "Sort by",
            match self.sort_by {
                SortBy::Metric => "Metric",
                SortBy::Date => "Date",
            },
        ));
        chips.push(chip(
            "order",
            "Order",
            match self.order {
                SortOrder::Asc => "Ascending",
                SortOrder::Desc => "Descending",
            },
        ));
        if let Some(limit) = self.limit {
            chips.push(chip("limit", "Limit", limit.value().to_string()));
        }
        chips
    }
}

fn chip(key: &str, label: &str, value: impl Into<String>) -> Chip {
    Chip {
        key: key.into(),
        label: label.into(),
        value: value.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> Query {
        Query {
            metric: Metric::Plays,
            entity: Some(Entity {
                kind: EntityKind::Artist,
                name: "Björk".into(),
                id: None,
            }),
            scope: None,
            period: Period::ThisYear,
            year: None,
            month: None,
            weekday: None,
            hour: None,
            source: None,
            from: None,
            to: None,
            group_by: None,
            sort_by: SortBy::Metric,
            order: SortOrder::Desc,
            limit: None,
        }
    }

    #[test]
    fn chips_are_deterministic_and_cover_the_interpretation() {
        let chips = query().chips();
        assert_eq!(
            chips
                .iter()
                .map(|chip| chip.key.as_str())
                .collect::<Vec<_>>(),
            ["metric", "entity", "period", "sort_by", "order",]
        );
        assert_eq!(chips[1].label, "Artist");
        assert_eq!(chips[1].value, "Björk");
        assert_eq!(chips[2].value, "This year");
    }

    #[test]
    fn validation_rejects_out_of_bounds_fields() {
        let mut invalid = query();
        invalid.month = Some(13);
        assert!(matches!(invalid.validate(), Err(AppError::BadRequest(_))));
        invalid.month = None;
        invalid.hour = Some(24);
        assert!(matches!(invalid.validate(), Err(AppError::BadRequest(_))));
        invalid.hour = None;
        invalid.entity = None;
        invalid.source = Some("  ".into());
        assert!(matches!(invalid.validate(), Err(AppError::BadRequest(_))));
        assert!(query().validate().is_ok());
    }

    #[test]
    fn every_limit_stays_within_the_declared_maximum() {
        for limit in [
            Limit::One,
            Limit::Five,
            Limit::Ten,
            Limit::TwentyFive,
            Limit::Fifty,
            Limit::Hundred,
        ] {
            assert!((1..=Limit::MAX).contains(&limit.value()));
        }
    }
}

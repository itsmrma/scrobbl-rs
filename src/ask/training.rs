//! Supervised fine-tuning data for the local Ask SQL model.
//!
//! `evals/ask/train_templates.json` holds question templates (English and
//! Italian paraphrases), the reference SQL that answers them and the answer
//! metadata. [`generate`] expands every template with synthetic values (artist
//! and track names, years, devices, countries, Spotify start/end reasons, ...)
//! and a varying "today", then renders each example exactly as the runtime
//! will see it: the production prompt from [`crate::ask::prompt`] as the
//! prompt, and the JSON the grammar allows plus `<|im_end|>` as the completion.
//!
//! Nothing here reads an archive: every value comes from the fixed pools below.
//! The questions of `evals/ask/sql_corpus.jsonl` are never emitted, so the
//! evaluation corpus stays a held-out test set. See `docs/ask-training.md`.
use crate::ask::{
    parser::{Previous, SqlPrevious},
    prompt,
};
use chrono::{Datelike, Duration, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const TEMPLATES: &str = include_str!("../../evals/ask/train_templates.json");
const EVAL_CORPUS: &str = include_str!("../../evals/ask/sql_corpus.jsonl");

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub id: String,
    pub tags: Vec<String>,
    pub kind: String,
    pub columns: Vec<(String, String)>,
    pub en: Vec<String>,
    pub it: Vec<String>,
    pub title: Localized,
    pub computed: Localized,
    pub sql: String,
    #[serde(default)]
    pub followup: Option<Followup>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Localized {
    pub en: String,
    pub it: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Followup {
    pub en: Vec<String>,
    pub it: Vec<String>,
}

impl Localized {
    fn get(&self, lang: &str) -> &str {
        if lang == "it" { &self.it } else { &self.en }
    }
}

/// One rendered training example.
#[derive(Clone, Debug)]
pub struct Example {
    pub id: String,
    pub template: String,
    pub lang: String,
    pub tags: Vec<String>,
    pub today: String,
    pub question: String,
    pub previous: Option<Previous>,
    pub title: String,
    pub computed: String,
    pub answer_kind: String,
    pub sql: String,
    pub columns: Vec<(String, String)>,
}

impl Example {
    /// The assistant turn, with fields in the order the SQL grammar emits.
    pub fn completion_json(&self) -> String {
        let columns: Vec<Value> = self
            .columns
            .iter()
            .map(|(name, format)| json!({"name": name, "format": format}))
            .collect();
        // Built by hand: serde_json maps would reorder the keys.
        format!(
            "{{\"title\":{},\"computed\":{},\"answer_kind\":{},\"sql\":{},\"columns\":{}}}",
            json!(self.title),
            json!(self.computed),
            json!(self.answer_kind),
            json!(self.sql),
            Value::Array(columns)
        )
    }

    /// The exact runtime prompt, ending with the empty Qwen3.5 thinking block.
    pub fn prompt(&self) -> String {
        prompt::build_sql_prompt_with_previous(&self.question, self.previous.as_ref(), &self.today)
    }

    /// Whether the example belongs to the small validation split.
    pub fn is_validation(&self) -> bool {
        fnv1a(&self.id).is_multiple_of(20)
    }

    pub fn record(&self) -> Value {
        json!({
            "id": self.id,
            "template": self.template,
            "lang": self.lang,
            "tags": self.tags,
            "today": self.today,
            "question": self.question,
            "previous": self.previous.as_ref().map(|p| json!({
                "question": p.question,
                "sql": p.sql.as_ref().map(|s| s.sql.clone()),
            })),
            "split": if self.is_validation() { "validation" } else { "train" },
            "prompt": self.prompt(),
            "completion": format!("{}<|im_end|>", self.completion_json()),
        })
    }
}

pub fn templates() -> Result<Vec<Template>, String> {
    let templates: Vec<Template> =
        serde_json::from_str(TEMPLATES).map_err(|e| format!("train_templates.json: {e}"))?;
    let mut ids = BTreeSet::new();
    for t in &templates {
        if !ids.insert(t.id.clone()) {
            return Err(format!("duplicate template id {}", t.id));
        }
        if t.en.is_empty() || t.it.is_empty() {
            return Err(format!("{} needs English and Italian questions", t.id));
        }
        let kinds = ["value", "table", "series", "ranking", "unsupported"];
        if !kinds.contains(&t.kind.as_str()) {
            return Err(format!("{}: unknown kind {}", t.id, t.kind));
        }
        let formats = [
            "count",
            "duration_ms",
            "date",
            "datetime",
            "percent",
            "text",
            "artist",
            "album",
            "track",
        ];
        if let Some((name, format)) = t
            .columns
            .iter()
            .find(|(_, f)| !formats.contains(&f.as_str()))
        {
            return Err(format!(
                "{}: column {name} has unknown format {format}",
                t.id
            ));
        }
        if (t.kind == "unsupported") != t.sql.is_empty() {
            return Err(format!("{}: only unsupported templates have no SQL", t.id));
        }
    }
    Ok(templates)
}

/// Questions of the evaluation corpus, normalised, so training never sees them.
pub fn held_out_questions() -> BTreeSet<String> {
    EVAL_CORPUS
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|v| v["question"].as_str().map(normalize))
        .collect()
}

fn normalize(text: &str) -> String {
    text.trim().to_lowercase()
}

/// Expand every template `samples` times per paraphrase and language.
pub fn generate(samples: usize, seed: u64) -> Result<Vec<Example>, String> {
    let held_out = held_out_questions();
    let mut rng = Rng(seed ^ 0x9e37_79b9_7f4a_7c15);
    let mut examples = Vec::new();
    let mut seen = BTreeSet::new();
    for template in templates()? {
        for lang in ["en", "it"] {
            let questions = if lang == "it" {
                &template.it
            } else {
                &template.en
            };
            for (q_index, question) in questions.iter().enumerate() {
                for sample in 0..samples.max(1) {
                    let params = Params::sample(&mut rng);
                    let example = render(&template, lang, question, &params, None)?;
                    let key = normalize(&example.question) + "\u{0}" + &example.today;
                    if held_out.contains(&normalize(&example.question)) || !seen.insert(key) {
                        continue;
                    }
                    let id = format!("{}/{lang}/{q_index}/{sample}", template.id);
                    examples.push(Example {
                        id: id.clone(),
                        ..example.clone()
                    });

                    if let Some(followup) = &template.followup {
                        let turns = if lang == "it" {
                            &followup.it
                        } else {
                            &followup.en
                        };
                        let turn = &turns[rng.below(turns.len())];
                        let mut next = params.clone();
                        next.values.insert("year".into(), params.get("year2"));
                        next.values
                            .insert("year_next".into(), params.get("year2_next"));
                        let previous = Previous {
                            question: example.question.clone(),
                            sql: Some(SqlPrevious {
                                title: example.title.clone(),
                                computed: example.computed.clone(),
                                sql: example.sql.clone(),
                            }),
                        };
                        let follow = render(&template, lang, turn, &params, Some(previous))
                            .and_then(|f| {
                                let base = render(&template, lang, question, &next, None)?;
                                Ok(Example {
                                    id: format!("{id}/followup"),
                                    title: base.title,
                                    computed: base.computed,
                                    sql: base.sql,
                                    ..f
                                })
                            })?;
                        examples.push(follow);
                    }
                }
            }
        }
    }
    Ok(examples)
}

fn render(
    template: &Template,
    lang: &str,
    question: &str,
    params: &Params,
    previous: Option<Previous>,
) -> Result<Example, String> {
    let fill = |text: &str, sql: bool| {
        params
            .fill(text, sql)
            .map_err(|e| format!("{}: {e}", template.id))
    };
    let mut tags = template.tags.clone();
    if previous.is_some() {
        tags.push("followup".into());
    }
    Ok(Example {
        id: String::new(),
        template: template.id.clone(),
        lang: lang.into(),
        tags,
        today: params.get("today"),
        question: fill(question, false)?,
        previous,
        title: fill(template.title.get(lang), false)?,
        computed: fill(template.computed.get(lang), false)?,
        answer_kind: template.kind.clone(),
        sql: fill(&template.sql, true)?,
        columns: template.columns.clone(),
    })
}

#[derive(Clone, Debug)]
struct Params {
    values: BTreeMap<String, String>,
}

impl Params {
    fn sample(rng: &mut Rng) -> Self {
        let mut v = BTreeMap::new();
        let mut set = |k: &str, value: String| {
            v.insert(k.to_string(), value);
        };
        let artist = ARTISTS[rng.below(ARTISTS.len())];
        let mut artist2 = ARTISTS[rng.below(ARTISTS.len())];
        while artist2 == artist {
            artist2 = ARTISTS[rng.below(ARTISTS.len())];
        }
        set("artist", artist.into());
        set("artist2", artist2.into());
        let (track, track_artist) = TRACKS[rng.below(TRACKS.len())];
        set("track", track.into());
        set("track_artist", track_artist.into());
        let (album, album_artist) = ALBUMS[rng.below(ALBUMS.len())];
        set("album", album.into());
        set("album_artist", album_artist.into());
        set("word", WORDS[rng.below(WORDS.len())].into());

        let year = 2019 + rng.below(7) as i32;
        let year2 = year + 1 + rng.below((2026 - year) as usize) as i32;
        set("year", year.to_string());
        set("year_next", (year + 1).to_string());
        set("year2", year2.to_string());
        set("year2_next", (year2 + 1).to_string());
        let month = 1 + rng.below(12) as u32;
        set("month", month.to_string());
        set("month_en", MONTHS_EN[month as usize - 1].into());
        set("month_it", MONTHS_IT[month as usize - 1].into());

        let day = NaiveDate::from_ymd_opt(2019, 1, 1).unwrap_or_default()
            + Duration::days(rng.below(365 * 7) as i64);
        set("date", day.format("%Y-%m-%d").to_string());
        set(
            "date_en",
            format!(
                "{} {} {}",
                day.day(),
                MONTHS_EN[day.month0() as usize],
                day.year()
            ),
        );
        set(
            "date_it",
            format!(
                "{} {} {}",
                day.day(),
                MONTHS_IT[day.month0() as usize],
                day.year()
            ),
        );

        let today = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap_or_default()
            + Duration::days(rng.below(700) as i64);
        let date = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
        set("today", date(today));
        set("yesterday", date(today - Duration::days(1)));
        set("this_year", today.year().to_string());
        set("last_year", (today.year() - 1).to_string());
        let month_start = today.with_day(1).unwrap_or(today);
        let last_month_end = month_start - Duration::days(1);
        set("this_month_start", date(month_start));
        set(
            "last_month_start",
            date(last_month_end.with_day(1).unwrap_or(last_month_end)),
        );
        set("last_month_end", date(last_month_end));
        let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
        set("this_week_start", date(monday));
        let days = [7_i64, 14, 30, 90, 365][rng.below(5)];
        set("days", days.to_string());
        set("days_ago", date(today - Duration::days(days - 1)));

        set("n", [3, 5, 10, 20][rng.below(4)].to_string());
        set("hour", rng.below(24).to_string());
        let weekday = 1 + rng.below(7);
        set("weekday", weekday.to_string());
        set("weekday_en", WEEKDAYS_EN[weekday - 1].into());
        set("weekday_it", WEEKDAYS_IT[weekday - 1].into());

        for (key, pool) in [
            ("platform", PLATFORMS),
            ("device_type", DEVICE_TYPES),
            ("country", COUNTRIES),
            ("reason_start", REASONS_START),
            ("reason_end", REASONS_END),
            ("source", SOURCES),
        ] {
            let (value, en, it) = pool[rng.below(pool.len())];
            set(key, value.into());
            set(&format!("{key}_en"), en.into());
            set(&format!("{key}_it"), it.into());
        }
        Self { values: v }
    }

    /// Replace `{name}` placeholders. Inside SQL, values are string-escaped.
    fn fill(&self, text: &str, sql: bool) -> Result<String, String> {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some((before, after)) = rest.split_once('{') {
            out.push_str(before);
            let (name, tail) = after
                .split_once('}')
                .ok_or_else(|| format!("unclosed placeholder in {text}"))?;
            let value = self
                .values
                .get(name)
                .ok_or_else(|| format!("unknown placeholder {{{name}}}"))?;
            if sql {
                out.push_str(&value.replace('\'', "''"));
            } else {
                out.push_str(value);
            }
            rest = tail;
        }
        out.push_str(rest);
        Ok(out)
    }

    fn get(&self, key: &str) -> String {
        self.values.get(key).cloned().unwrap_or_default()
    }
}

/// Small deterministic generator, so a seed always yields the same dataset.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next() % bound as u64) as usize
    }
}

fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

// Fixture names come first so part of the data returns rows on the synthetic
// archive; the rest are public artist names that exercise accents, non-Latin
// scripts and apostrophes in SQL literals.
const ARTISTS: &[&str] = &[
    "Bjork",
    "Ado",
    "BTS",
    "Halsey",
    "Radiohead",
    "Sigur Ros",
    "Björk",
    "Sigur Rós",
    "Massive Attack",
    "Daft Punk",
    "Rosalía",
    "Måneskin",
    "Lucio Battisti",
    "Mina",
    "Fabrizio De André",
    "Taylor Swift",
    "Kendrick Lamar",
    "Beyoncé",
    "Bad Bunny",
    "The Beatles",
    "Pink Floyd",
    "Billie Eilish",
    "Arctic Monkeys",
    "Vasco Rossi",
    "Laura Pausini",
    "Elodie",
    "Marracash",
    "米津玄師",
    "YOASOBI",
    "Guns N' Roses",
    "Florence + The Machine",
    "AC/DC",
    "Sawano Hiroyuki",
    "Linkin Park",
];

const TRACKS: &[(&str, &str)] = &[
    ("Joga", "Bjork"),
    ("Show", "Ado"),
    ("Odo", "Ado"),
    ("Boy With Luv", "BTS"),
    ("Mikrokosmos", "BTS"),
    ("Weird Fishes", "Radiohead"),
    ("Hoppipolla", "Sigur Ros"),
    ("Teardrop", "Massive Attack"),
    ("Get Lucky", "Daft Punk"),
    ("Malamente", "Rosalía"),
    ("Zitti e buoni", "Måneskin"),
    ("Emozioni", "Lucio Battisti"),
    ("Anti-Hero", "Taylor Swift"),
    ("HUMBLE.", "Kendrick Lamar"),
    ("Sweet Child O' Mine", "Guns N' Roses"),
    ("Lemon", "米津玄師"),
    ("Idol", "YOASOBI"),
    ("bad guy", "Billie Eilish"),
    ("Do I Wanna Know?", "Arctic Monkeys"),
    ("Albachiara", "Vasco Rossi"),
    ("La canzone di Marinella", "Fabrizio De André"),
    ("Numb", "Linkin Park"),
];

const ALBUMS: &[(&str, &str)] = &[
    ("Homogenic", "Bjork"),
    ("Zanmu", "Ado"),
    ("Persona", "BTS"),
    ("In Rainbows", "Radiohead"),
    ("Takk...", "Sigur Ros"),
    ("Mezzanine", "Massive Attack"),
    ("Random Access Memories", "Daft Punk"),
    ("El Mal Querer", "Rosalía"),
    ("Teatro d'ira: Vol. I", "Måneskin"),
    ("Anima latina", "Lucio Battisti"),
    ("Midnights", "Taylor Swift"),
    ("DAMN.", "Kendrick Lamar"),
    ("AM", "Arctic Monkeys"),
    ("The Dark Side of the Moon", "Pink Floyd"),
    ("Abbey Road", "The Beatles"),
];

const WORDS: &[&str] = &["love", "night", "amore", "rain", "live", "remix", "home"];

const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_IT: [&str; 12] = [
    "gennaio",
    "febbraio",
    "marzo",
    "aprile",
    "maggio",
    "giugno",
    "luglio",
    "agosto",
    "settembre",
    "ottobre",
    "novembre",
    "dicembre",
];
const WEEKDAYS_EN: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const WEEKDAYS_IT: [&str; 7] = [
    "lunedì",
    "martedì",
    "mercoledì",
    "giovedì",
    "venerdì",
    "sabato",
    "domenica",
];

type Choice = (&'static str, &'static str, &'static str);

const PLATFORMS: &[Choice] = &[
    ("android", "Android", "Android"),
    ("ios", "my iPhone", "iPhone"),
    ("windows", "my Windows PC", "PC Windows"),
    ("macos", "my Mac", "Mac"),
    ("linux", "Linux", "Linux"),
    ("web", "the web player", "web player"),
    ("xbox", "the Xbox", "Xbox"),
    ("playstation", "the PlayStation", "PlayStation"),
    ("alexa", "Alexa", "Alexa"),
    ("google_cast", "Chromecast", "Chromecast"),
    ("sonos", "Sonos", "Sonos"),
    ("tv", "the smart TV", "smart TV"),
];
const DEVICE_TYPES: &[Choice] = &[
    ("phone", "phone", "telefono"),
    ("computer", "computer", "computer"),
    ("tablet", "tablet", "tablet"),
    ("console", "game console", "console"),
    ("speaker", "smart speaker", "smart speaker"),
    ("tv", "TV", "televisore"),
    ("web", "web player", "web player"),
    ("car", "car", "sistema dell'auto"),
];
const COUNTRIES: &[Choice] = &[
    ("IT", "Italy", "Italia"),
    ("DE", "Germany", "Germania"),
    ("AT", "Austria", "Austria"),
    ("US", "the United States", "Stati Uniti"),
    ("GB", "the United Kingdom", "Regno Unito"),
    ("FR", "France", "Francia"),
    ("ES", "Spain", "Spagna"),
    ("JP", "Japan", "Giappone"),
];
const REASONS_START: &[Choice] = &[
    (
        "trackdone",
        "the previous song ended",
        "è finito il brano precedente",
    ),
    (
        "clickrow",
        "I picked it from a list",
        "l'ho scelto da una lista",
    ),
    ("fwdbtn", "I pressed next", "ho premuto avanti"),
    ("backbtn", "I pressed back", "ho premuto indietro"),
    ("playbtn", "I pressed play", "ho premuto play"),
    (
        "remote",
        "another device started it",
        "l'ha avviato un altro dispositivo",
    ),
    ("appload", "the app opened", "si è aperta l'app"),
];
const REASONS_END: &[Choice] = &[
    (
        "trackdone",
        "the song played to the end",
        "il brano è arrivato alla fine",
    ),
    (
        "endplay",
        "I stopped playback",
        "ho fermato la riproduzione",
    ),
    (
        "fwdbtn",
        "I skipped to the next song",
        "sono passato al brano successivo",
    ),
    ("backbtn", "I went back", "sono tornato indietro"),
    ("logout", "I logged out", "ho fatto logout"),
    (
        "remote",
        "another device took over",
        "un altro dispositivo ha preso il controllo",
    ),
    (
        "unexpected-exit",
        "the app closed unexpectedly",
        "l'app si è chiusa all'improvviso",
    ),
];
const SOURCES: &[Choice] = &[
    (
        "spotify_import",
        "the Spotify extended history import",
        "l'import della cronologia estesa di Spotify",
    ),
    (
        "spotify_api",
        "the Spotify recent listening collection",
        "la raccolta degli ascolti recenti di Spotify",
    ),
    ("listenbrainz", "ListenBrainz", "ListenBrainz"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ask::{
        fixture, projection,
        sandbox::{self, SandboxLimits},
    };

    #[test]
    fn templates_parse_and_every_placeholder_resolves() {
        let examples = generate(1, 7).expect("templates expand");
        assert!(examples.len() > 500, "only {} examples", examples.len());
        assert!(examples.iter().any(|e| e.previous.is_some()));
        assert!(examples.iter().any(|e| e.lang == "it"));
    }

    #[test]
    fn the_same_seed_gives_the_same_dataset() {
        let a: Vec<String> = generate(1, 3)
            .unwrap()
            .iter()
            .map(|e| e.completion_json())
            .collect();
        let b: Vec<String> = generate(1, 3)
            .unwrap()
            .iter()
            .map(|e| e.completion_json())
            .collect();
        assert_eq!(a, b);
    }

    #[test]
    fn evaluation_questions_never_enter_the_training_data() {
        let held_out = held_out_questions();
        assert!(!held_out.is_empty());
        for example in generate(2, 11).unwrap() {
            assert!(
                !held_out.contains(&normalize(&example.question)),
                "{} leaks an evaluation question",
                example.id
            );
        }
    }

    #[test]
    fn completions_follow_the_grammar_field_order_and_schema() {
        for example in generate(1, 5).unwrap() {
            let json = example.completion_json();
            assert!(json.starts_with("{\"title\":"), "{json}");
            let parsed: crate::ask::schema::SqlModelOutput =
                serde_json::from_str(&json).expect("completion matches SqlModelOutput");
            assert_eq!(parsed.columns.len(), example.columns.len());
            assert!(example.prompt().ends_with(prompt::THINKING_PREFIX));
        }
    }

    #[test]
    fn every_projection_column_is_taught_by_some_template() {
        let sql: String = templates()
            .unwrap()
            .iter()
            .map(|t| t.sql.to_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        let word = |name: &str| {
            sql.match_indices(name).any(|(at, _)| {
                let before = sql[..at].chars().next_back();
                let after = sql[at + name.len()..].chars().next();
                !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                    && !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
            })
        };
        for table in projection::TABLES {
            assert!(word(table.name), "no template reads table {}", table.name);
            for column in table.columns {
                assert!(
                    word(column.name),
                    "no template uses {}.{}",
                    table.name,
                    column.name
                );
            }
        }
    }

    #[tokio::test]
    async fn every_generated_query_runs_in_the_sandbox_with_its_declared_columns() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = fixture::build(dir.path()).await.unwrap();
        let mut checked = BTreeSet::new();
        for example in generate(1, 13).unwrap() {
            if example.sql.is_empty() || !checked.insert(example.sql.clone()) {
                continue;
            }
            let output = sandbox::execute(
                fixture.projection_path(),
                &example.sql,
                SandboxLimits::default(),
            )
            .unwrap_or_else(|rejection| {
                panic!(
                    "{}: {} rejected: {}",
                    example.id, example.sql, rejection.model_error
                )
            });
            let declared: Vec<&str> = example.columns.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(output.columns, declared, "{}: {}", example.id, example.sql);
        }
        assert!(
            checked.len() > 300,
            "only {} distinct queries",
            checked.len()
        );
    }
}

//! Compact system prompt for local Ask SQL generation and repair.
//!
//! `llama-cpp-2` cannot pass `chat_template_kwargs`, so the ChatML envelope
//! and empty Qwen3.5 thinking block are written here. Pre-filling
//! `<|im_start|>assistant\n<think>\n\n</think>\n\n` disables Qwen3.5 thinking
//! mode cleanly for Qwen3.5 (the curated 2B and its SQL fine-tune), and
//! the GBNF grammar forces valid JSON matching
//! [`crate::ask::schema::sql_schema`].
use crate::ask::{parser::Previous, projection, sandbox};

pub const THINKING_PREFIX: &str = "<|im_start|>assistant\n<think>\n\n</think>\n\n";

/// Build the ChatML prompt for a standalone local SQL question.
pub fn build_sql_prompt(question: &str, today: &str) -> String {
    build_sql_prompt_with_previous(question, None, today)
}

/// Build the ChatML prompt for a local SQL question, optionally carrying the
/// previous question and its generated SQL (never previous archive rows).
pub fn build_sql_prompt_with_previous(
    question: &str,
    previous: Option<&Previous>,
    today: &str,
) -> String {
    format!(
        "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n{}",
        local_sql_system_prompt(today),
        sql_user_turn(question, previous),
        THINKING_PREFIX
    )
}

/// System prompt for local Ask SQL generation: carries only the documented
/// projection schema, counting rules, today's date, and examples — never any
/// archive values.
pub fn local_sql_system_prompt(today: &str) -> String {
    sql_system_prompt_with_contract(
        today,
        "Output JSON only with fields in order: title, computed, answer_kind (value|table|series|ranking|unsupported), sql, columns ([{name, format}] where format is count|duration_ms|date|datetime|percent|text|artist|album|track).",
    )
}

/// Build the ChatML prompt for a local SQL repair round. It keeps the local SQL
/// contract and adds the previous answer and every failed SQL with the error the
/// sandbox returned for it; results and archive values are never included.
pub fn build_sql_repair_prompt(
    question: &str,
    previous: Option<&Previous>,
    attempts: &[(&str, &str)],
    today: &str,
) -> String {
    format!(
        "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\nA previous SQL query for this question was rejected by the sandbox. Write a corrected query, still answering the same question.\n\n{}<|im_end|>\n{}",
        local_sql_system_prompt(today),
        sql_repair_user_turn(question, previous, attempts),
        THINKING_PREFIX
    )
}

fn push_previous_context(text: &mut String, previous: Option<&Previous>) {
    if let Some(previous) = previous
        && let Some(sql) = previous.sql.as_ref().filter(|s| !s.sql.trim().is_empty())
    {
        text.push_str(&format!(
            "Previous question: {}\n",
            previous.question.trim()
        ));
        if !sql.computed.trim().is_empty() {
            text.push_str(&format!("Previous computed: {}\n", sql.computed.trim()));
        }
        text.push_str(&format!("Previous SQL: {}\n\n", sql.sql.trim()));
    }
}

pub fn sql_user_turn_for_openai(question: &str, previous: Option<&Previous>) -> String {
    sql_user_turn(question, previous)
}

fn sql_user_turn(question: &str, previous: Option<&Previous>) -> String {
    if previous.is_none_or(|p| p.sql.as_ref().is_none_or(|s| s.sql.trim().is_empty())) {
        return question.trim().to_owned();
    }
    let mut text = String::new();
    push_previous_context(&mut text, previous);
    text.push_str(&format!("Question: {}", question.trim()));
    text
}

pub fn sql_repair_user_turn(
    question: &str,
    previous: Option<&Previous>,
    attempts: &[(&str, &str)],
) -> String {
    let mut text = String::new();
    push_previous_context(&mut text, previous);
    text.push_str(&format!("Question: {}\n", question.trim()));
    for (index, (sql, error)) in attempts.iter().enumerate() {
        let clipped: String = sql.chars().take(sandbox::MAX_SQL_BYTES).collect();
        text.push_str(&format!(
            "\nAttempt {} failed.\nSQL: {clipped}\nError: {error}\n",
            index + 1
        ));
    }
    text.push_str(
        "\nFix the SQL query carefully using exact schema table/column names (`tracks.title`, `artists.name`, `albums.name`, `listens.local_*`; join `track_artists` for artists and `tracks` for tracks) and single quotes `'...'` for strings. \
         Return the corrected JSON object, or set answer_kind to \"unsupported\" if unanswerable from this schema.",
    );
    text
}

fn sql_system_prompt_with_contract(today: &str, contract_line: &str) -> String {
    let schema = projection::schema_description();
    format!(
        "Translate one personal music-listening question into one read-only SQLite SELECT (or WITH ... SELECT) query.\n\
         The archive is never shown to you; the app executes your SQL in a strict read-only sandbox.\n\
         Today is {today}.\n\n\
         {schema}\n\n\
         Rules:\n\
         - Filter `counted = 1` for plays/stats unless asked for skipped/short plays (`counted = 0`) or all attempts. Only add a date cutoff (`>= date(...)`) when the question specifies a time period or recent window.\n\
         - Output shape: \"how many ...\" / \"quanti ...\" (artists, songs, days, plays) must return ONE scalar row (`COUNT(...) AS ...`), not a multi-row list. \"How many days\" / \"in quanti giorni\" -> `COUNT(DISTINCT l.local_date) AS days`. \"How many different songs\" / \"brani diversi\" -> `COUNT(DISTINCT t.edition_key) AS songs`.\n\
         - Query `listens` directly without joins whenever no artist, track or album filter/grouping is asked (e.g. total listening time or plays by device/offline/country/hour). Otherwise join:\n\
           * Artist: `listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id` with `COUNT(DISTINCT l.id)` to avoid double-counting multi-artist tracks.\n\
           * Track or `<track> by <artist>` / `<brano> di <artista>`: `listens l JOIN tracks t ON t.id = l.track_id` (plus `track_artists` + `artists` when an artist is named, splitting `<track>` into `t.title` and `<artist>` into `a.name`). When only one artist name appears ('ascolti di X', 'plays of X') without a separate track title, filter only `a.name`.\n\
           * Album: `LEFT JOIN albums al ON al.id = t.album_id`; use `album_artists` for main album artists.\n\
         - Comparing named entities ('compare A and B', 'confronta A e B'): return one row per entity (`SELECT a.name AS artist, COUNT(DISTINCT l.id) AS plays ... WHERE l.counted = 1 AND LOWER(a.name) IN (LOWER('A'), LOWER('B')) GROUP BY a.id ORDER BY plays DESC, a.name ASC`).\n\
         - Repeated plays on the same day ('more than once on the same day' / 'più di una volta nello stesso giorno'): `SELECT t.title AS track, l.local_date AS day, COUNT(*) AS plays FROM listens l JOIN tracks t ON t.id = l.track_id WHERE l.counted = 1 GROUP BY t.id, l.local_date HAVING COUNT(*) > 1 ORDER BY l.local_date, track`.\n\
         - Discovery / lifecycle ('first played in Y1 and still played in Y2' / 'ascoltati per la prima volta nel Y1 e ancora nel Y2'): compute per-artist bounds first (`WITH artist_span AS (SELECT a.id, MIN(l.local_date) AS first_day, MAX(l.local_date) AS last_day FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id WHERE l.counted = 1 GROUP BY a.id) SELECT COUNT(*) AS artists FROM artist_span WHERE first_day >= 'Y1-01-01' AND first_day < 'Y2-01-01' AND last_day >= 'Y2-01-01'`).\n\
         - Duration: `listened_ms` is NULL only when `duration_kind = 'unknown'`. Use `SUM(listened_ms)` with `WHERE counted = 1` for total known listening time (this covers both `'actual'` and `'estimated'` known durations; filter `duration_kind = 'actual'` only if asked for actual-only).\n\
         - Context columns (`platform`, `device_type`, `country`, `shuffle`, `skipped`, `offline`, `reason_start`, `reason_end`) are NULL when unknown: add `IS NOT NULL` when grouping/counting known values. Phone/computer/tablet/console/speaker/TV questions use `device_type`; Android/iPhone/Windows/Mac/Linux/web/Xbox/Alexa questions use `platform`; never filter the raw `device` string. Boolean flags are `1`/`0`; `skipped = 1` means Spotify marked the play as skipped. \"After midnight\" / \"dopo mezzanotte\" means `local_hour = 0`. Filter `l.sources` via `EXISTS (SELECT 1 FROM json_each(l.sources) WHERE value = 'spotify_import')`.\n\
         - Dates & streaks: use `calendar` for gap coverage; `CAST(julianday(MAX(l.local_date)) - julianday(MIN(l.local_date)) AS INTEGER) AS days` for days between first and last play. Longest consecutive-day streak: `WITH days AS (SELECT DISTINCT l.local_date AS day FROM ... WHERE ...), runs AS (SELECT day, julianday(day) - ROW_NUMBER() OVER (ORDER BY day) AS grp FROM days) SELECT MAX(run_length) AS longest_streak FROM (SELECT COUNT(*) AS run_length FROM runs GROUP BY grp)`.\n\
         - Top-N with ties: `WITH ranked AS (...), positioned AS (SELECT name, plays, DENSE_RANK() OVER (ORDER BY plays DESC) AS position FROM ranked) SELECT name, plays FROM positioned WHERE position <= N ORDER BY plays DESC, name ASC`.\n\
         - Strings & SQL safety: use single quotes `'...'` for string literals; match names case-insensitively (`LOWER(x) = LOWER('...')` or `fold(...)`) without altering spelling. One `SELECT` or `WITH`, no parameters/PRAGMA/`sqlite_*`.\n\
         - {contract_line} Keep `columns` in exact `SELECT` alias order. Always write `title` and `computed` in the same language as the question; state metric, filters, period and grouping concisely in `computed`, then write `sql`. For follow-ups building on `Previous question`/`Previous SQL`, carry forward relevant constraints; ignore unrelated previous turns. If unrelated to listening history or requesting writes, set `answer_kind` to \"unsupported\", `sql` to \"\", `columns` to [].\n\n\
         Examples:\n\
         Q: How many times did I listen to Björk?\n\
         A: {{\"title\":\"Plays for Björk\",\"computed\":\"Counted plays for artist Björk across all time.\",\"answer_kind\":\"value\",\"sql\":\"SELECT COUNT(DISTINCT l.id) AS plays FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id WHERE l.counted = 1 AND LOWER(a.name) = LOWER('Björk')\",\"columns\":[{{\"name\":\"plays\",\"format\":\"count\"}}]}}\n\
         Q: Quando ho ascoltato per la prima volta Teardrop dei Massive Attack?\n\
         A: {{\"title\":\"Primo ascolto di Teardrop (Massive Attack)\",\"computed\":\"Prima data di ascolto conteggiato per il brano Teardrop di Massive Attack.\",\"answer_kind\":\"value\",\"sql\":\"SELECT MIN(l.local_date) AS first_play FROM listens l JOIN tracks t ON t.id = l.track_id JOIN track_artists ta ON ta.track_id = t.id JOIN artists a ON a.id = ta.artist_id WHERE l.counted = 1 AND LOWER(t.title) = LOWER('Teardrop') AND LOWER(a.name) = LOWER('Massive Attack')\",\"columns\":[{{\"name\":\"first_play\",\"format\":\"date\"}}]}}\n\
         Q: Top 5 tracks in 2024\n\
         A: {{\"title\":\"Top 5 tracks in 2024\",\"computed\":\"Top 5 tracks by counted plays in 2024.\",\"answer_kind\":\"ranking\",\"sql\":\"SELECT t.title AS track, COUNT(*) AS plays FROM listens l JOIN tracks t ON t.id = l.track_id WHERE l.counted = 1 AND l.local_year = 2024 GROUP BY t.id ORDER BY plays DESC, t.title ASC LIMIT 5\",\"columns\":[{{\"name\":\"track\",\"format\":\"track\"}},{{\"name\":\"plays\",\"format\":\"count\"}}]}}\n\
         Q: Qual è il mio artista più ascoltato?\n\
         A: {{\"title\":\"Artista più ascoltato\",\"computed\":\"Artista con il maggior numero di ascolti conteggiati in tutto il periodo.\",\"answer_kind\":\"table\",\"sql\":\"SELECT a.name AS artist, COUNT(DISTINCT l.id) AS plays FROM listens l JOIN track_artists ta ON ta.track_id = l.track_id JOIN artists a ON a.id = ta.artist_id WHERE l.counted = 1 GROUP BY a.id ORDER BY plays DESC, a.name ASC LIMIT 1\",\"columns\":[{{\"name\":\"artist\",\"format\":\"artist\"}},{{\"name\":\"plays\",\"format\":\"count\"}}]}}\n\
         Q: What is the weather today?\n\
         A: {{\"title\":\"\",\"computed\":\"I can only answer questions about your listening history.\",\"answer_kind\":\"unsupported\",\"sql\":\"\",\"columns\":[]}}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ask::parser::SqlPrevious;

    #[test]
    fn local_sql_prompt_includes_projection_schema_today_and_thinking_prefix() {
        let prompt = build_sql_prompt("What are my top tracks?", "2026-11-16");
        assert!(prompt.ends_with(THINKING_PREFIX), "{prompt}");
        assert!(prompt.contains("2026-11-16"), "{prompt}");
        assert!(prompt.contains("listens("), "{prompt}");
        assert!(prompt.contains("counted = 1"), "{prompt}");
        assert!(prompt.contains("What are my top tracks?"), "{prompt}");
        assert!(prompt.contains("Björk"), "{prompt}");
        assert!(prompt.contains("Qual è il mio artista"), "{prompt}");
    }

    #[test]
    fn local_sql_followup_prompt_carries_previous_sql_but_not_results() {
        let prev = Previous {
            question: "How many plays did I have in 2023?".into(),
            sql: Some(SqlPrevious {
                title: "Plays in 2023".into(),
                computed: "Counted plays in 2023.".into(),
                sql:
                    "SELECT COUNT(*) AS plays FROM listens WHERE counted = 1 AND local_year = 2023"
                        .into(),
            }),
        };
        let prompt = build_sql_prompt_with_previous("And in 2024?", Some(&prev), "2026-11-16");
        assert!(prompt.contains("Previous question: How many plays did I have in 2023?"));
        assert!(prompt.contains("local_year = 2023"));
        assert!(prompt.contains("Question: And in 2024?"));
    }
}

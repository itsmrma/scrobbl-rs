//! Curated model catalog.
//!
//! Every downloadable entry is pinned to a repository revision, a file name,
//! its exact byte size and its LFS SHA-256. The values below were read from the
//! Hugging Face API when this module was written; never replace them with a
//! guessed checksum. URLs are derived from the pinned fields, never supplied by
//! a client.
use std::sync::{LazyLock, Mutex};

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct License {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug)]
pub struct DownloadSource {
    pub repo: String,
    pub revision: String,
    pub file: String,
    pub sha256: String,
    pub size_bytes: u64,
    /// Only used by tests that serve a synthetic file from a local HTTP server.
    /// Production entries derive their URL from the pinned repository fields.
    pub source_url: Option<String>,
}

impl DownloadSource {
    pub fn url(&self) -> String {
        self.source_url.clone().unwrap_or_else(|| {
            format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                self.repo, self.revision, self.file
            )
        })
    }
}

#[derive(Clone, Debug)]
pub struct Requirements {
    pub disk_bytes: u64,
    pub ram_min_bytes: u64,
    pub ram_recommended_bytes: u64,
    pub cpu_cores: u32,
    pub gpu_optional: bool,
    pub context_tokens: u32,
}

#[derive(Clone, Debug)]
pub struct LanguageScore {
    pub language: String,
    pub exact_match: Option<f64>,
    pub cases: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct ParserScores {
    pub intent: Option<f64>,
    pub slots: Option<f64>,
    pub refusals: Option<f64>,
    pub exact_match: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct MeasuredSpeed {
    pub p50_ms: u32,
    pub p95_ms: u32,
    pub ttft_ms: u32,
    pub decode_tokens_per_second: f64,
    pub prefill_tokens_per_second: f64,
    pub measured_on: String,
}

#[derive(Clone, Debug)]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    pub tier: String,
    pub status: String,
    pub description: String,
    pub download: DownloadSource,
    pub license: License,
    pub requirements: Requirements,
    pub languages: Vec<LanguageScore>,
    pub parser: ParserScores,
    pub speed: Option<MeasuredSpeed>,
    pub backend: String,
    pub selectable: bool,
}

impl CatalogModel {
    pub fn is_available(&self) -> bool {
        self.selectable && self.status == "available"
    }
}

fn language(language: &str, exact_match: Option<f64>, cases: Option<u32>) -> LanguageScore {
    LanguageScore {
        language: language.to_string(),
        exact_match,
        cases,
    }
}

fn repo_license(repo: &str) -> License {
    License {
        name: "Apache-2.0".to_string(),
        url: format!("https://huggingface.co/{repo}"),
    }
}

fn empty_parser() -> ParserScores {
    ParserScores {
        intent: None,
        slots: None,
        refusals: None,
        exact_match: None,
    }
}

/// Scores measured by `ask-eval` on `evals/ask/corpus.jsonl` (#181).
/// `slots` is the slot F1 and `refusals` the refusal accuracy.
fn measured_parser(intent: f64, slots: f64, refusals: f64, exact_match: f64) -> ParserScores {
    ParserScores {
        intent: Some(intent),
        slots: Some(slots),
        refusals: Some(refusals),
        exact_match: Some(exact_match),
    }
}

#[allow(clippy::too_many_arguments)]
fn qwen(
    id: &str,
    name: &str,
    tier: &str,
    status: &str,
    description: &str,
    repo: &str,
    revision: &str,
    file: &str,
    sha256: &str,
    size_bytes: u64,
    ram_min_bytes: u64,
    ram_recommended_bytes: u64,
    cpu_cores: u32,
    gpu_optional: bool,
    languages: Vec<LanguageScore>,
    parser: ParserScores,
    speed: Option<MeasuredSpeed>,
) -> CatalogModel {
    CatalogModel {
        id: id.to_string(),
        name: name.to_string(),
        tier: tier.to_string(),
        status: status.to_string(),
        description: description.to_string(),
        download: DownloadSource {
            repo: repo.to_string(),
            revision: revision.to_string(),
            file: file.to_string(),
            sha256: sha256.to_string(),
            size_bytes,
            source_url: None,
        },
        license: repo_license(repo),
        requirements: Requirements {
            disk_bytes: size_bytes,
            ram_min_bytes,
            ram_recommended_bytes,
            cpu_cores,
            gpu_optional,
            context_tokens: 2048,
        },
        languages,
        parser,
        speed,
        backend: "llama.cpp (CPU)".to_string(),
        selectable: status == "available",
    }
}

fn desktop_speed(
    p50_ms: u32,
    p95_ms: u32,
    ttft_ms: u32,
    decode_tokens_per_second: f64,
    prefill_tokens_per_second: f64,
) -> Option<MeasuredSpeed> {
    Some(MeasuredSpeed {
        p50_ms,
        p95_ms,
        ttft_ms,
        decode_tokens_per_second,
        prefill_tokens_per_second,
        measured_on: "Windows desktop, AMD Ryzen 7 5700X, 8 threads".to_string(),
    })
}

static BASE: LazyLock<Vec<CatalogModel>> = LazyLock::new(|| {
    vec![
        qwen(
            "qwen3-0.6b-q4_k_m",
            "Qwen3 0.6B",
            "light",
            "available",
            "Fastest option for short questions. Measured accuracy is well below the default, so treat the interpretation chips as part of the answer.",
            // The official Qwen repository publishes only Q8_0 for this size.
            // The spike measured this pinned Q4_K_M build; the checksum and
            // size below are the real Hugging Face LFS values.
            "unsloth/Qwen3-0.6B-GGUF",
            "50968a4468ef4233ed78cd7c3de230dd1d61a56b",
            "Qwen3-0.6B-Q4_K_M.gguf",
            "ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a",
            396_705_472,
            GIB,
            2 * GIB,
            2,
            false,
            vec![
                language("en", Some(0.4458), Some(104)),
                language("it", Some(0.3614), Some(104)),
            ],
            measured_parser(0.5577, 0.6111, 0.5817, 0.4036),
            desktop_speed(1904, 2416, 909, 48.9, 777.0),
        ),
        qwen(
            "qwen3-1.7b-q4_k_m",
            "Qwen3 1.7B",
            "default",
            "available",
            "Provisional default: measured accuracy is below the 1.7B eval target, so treat the interpretation chips as part of the answer. Use the 4B model when accuracy matters.",
            // The official Qwen repository publishes only Q8_0 for this size.
            // The spike measured this pinned Q4_K_M build.
            "ggml-org/Qwen3-1.7B-GGUF",
            "daeb8e2d528a760970442092f6bf1e55c3b659eb",
            "Qwen3-1.7B-Q4_K_M.gguf",
            "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5",
            1_282_439_264,
            3 * GIB,
            4 * GIB,
            4,
            false,
            vec![
                language("en", Some(0.7108), Some(104)),
                language("it", Some(0.6024), Some(104)),
            ],
            measured_parser(0.7885, 0.8376, 0.8846, 0.6566),
            desktop_speed(4138, 4611, 2352, 23.0, 296.0),
        ),
        qwen(
            "qwen3-4b-q4_k_m",
            "Qwen3 4B",
            "quality",
            "available",
            "Highest measured accuracy in the CPU tier and the closest to the eval targets, but it still misses the 4B intent and refusal targets and is roughly twice as slow as the 1.7B model.",
            "Qwen/Qwen3-4B-GGUF",
            "bc640142c66e1fdd12af0bd68f40445458f3869b",
            "Qwen3-4B-Q4_K_M.gguf",
            "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5",
            2_497_280_256,
            5 * GIB,
            8 * GIB,
            4,
            false,
            vec![
                language("en", Some(0.9036), Some(104)),
                language("it", Some(0.8675), Some(104)),
            ],
            measured_parser(0.9279, 0.9654, 0.9423, 0.8855),
            desktop_speed(8883, 9825, 5809, 10.6, 120.0),
        ),
        qwen(
            "qwen3-8b-q4_k_m",
            "Qwen3 8B",
            "gpu",
            "gpu_future",
            "GPU tier reserved for a later release. It is listed for planning only and cannot be selected or downloaded in v1.",
            "Qwen/Qwen3-8B-GGUF",
            "7c41481f57cb95916b40956ab2f0b139b296d974",
            "Qwen3-8B-Q4_K_M.gguf",
            "d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785",
            5_027_783_488,
            9 * GIB,
            16 * GIB,
            6,
            true,
            vec![language("en", None, None), language("it", None, None)],
            empty_parser(),
            None,
        ),
        qwen(
            "qwen3-32b-q4_k_m",
            "Qwen3 32B",
            "gpu",
            "gpu_future",
            "27B-class GPU model reserved for a later release. It is listed for planning only and cannot be selected or downloaded in v1.",
            "Qwen/Qwen3-32B-GGUF",
            "938a7432affaec9157f883a87164e2646ae17555",
            "Qwen3-32B-Q4_K_M.gguf",
            "efd971561896866f0e910cce52761ca77b1b138090c7f15fe284676d57d1f689",
            19_762_149_024,
            24 * GIB,
            32 * GIB,
            8,
            true,
            vec![language("en", None, None), language("it", None, None)],
            empty_parser(),
            None,
        ),
    ]
});

static TEST_MODELS: LazyLock<Mutex<Vec<CatalogModel>>> = LazyLock::new(|| Mutex::new(Vec::new()));

/// Replaces the curated list for a test id. Integration tests use it to point a
/// download at a local HTTP server instead of downloading a real model.
#[doc(hidden)]
pub fn register_test_model(model: CatalogModel) {
    if let Ok(mut models) = TEST_MODELS.lock() {
        models.retain(|existing| existing.id != model.id);
        models.push(model);
    }
}

#[doc(hidden)]
pub fn clear_test_models() {
    if let Ok(mut models) = TEST_MODELS.lock() {
        models.clear();
    }
}

pub fn all() -> Vec<CatalogModel> {
    let mut models = BASE.clone();
    if let Ok(tests) = TEST_MODELS.lock() {
        models.extend(tests.iter().cloned());
    }
    models
}

pub fn find(id: &str) -> Option<CatalogModel> {
    if let Ok(tests) = TEST_MODELS.lock()
        && let Some(model) = tests.iter().find(|model| model.id == id)
    {
        return Some(model.clone());
    }
    BASE.iter().find(|model| model.id == id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_has_a_pinned_checksum_and_size() {
        for model in all() {
            assert_eq!(
                model.download.sha256.len(),
                64,
                "{} must carry a SHA-256",
                model.id
            );
            assert!(model.download.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(model.download.size_bytes > 0);
            assert_eq!(model.requirements.disk_bytes, model.download.size_bytes);
            assert!(!model.download.revision.is_empty());
            assert!(model.download.url().starts_with("https://huggingface.co/"));
        }
    }

    #[test]
    fn only_the_three_cpu_tiers_are_selectable() {
        let selectable: Vec<String> = all()
            .into_iter()
            .filter(|model| model.is_available())
            .map(|model| model.tier)
            .collect();
        assert_eq!(selectable, ["light", "default", "quality"]);
        let future: Vec<_> = all()
            .into_iter()
            .filter(|model| model.status == "gpu_future")
            .collect();
        assert_eq!(future.len(), 2);
        assert!(future.iter().all(|model| !model.selectable));
    }
}

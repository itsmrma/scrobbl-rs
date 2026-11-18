//! Curated local SQL model catalog.
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
    pub speed: Option<MeasuredSpeed>,
    pub backend: String,
    pub selectable: bool,
}

impl CatalogModel {
    pub fn is_available(&self) -> bool {
        self.selectable && self.status == "available"
    }
}

fn repo_license(repo: &str) -> License {
    License {
        name: "Apache-2.0".to_string(),
        url: format!("https://huggingface.co/{repo}"),
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
            context_tokens: 4096,
        },
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
            "Fastest local SQL option for simple archive lookups. Smallest footprint, though complex multi-join queries may need SQL repair or a larger model.",
            "unsloth/Qwen3-0.6B-GGUF",
            "50968a4468ef4233ed78cd7c3de230dd1d61a56b",
            "Qwen3-0.6B-Q4_K_M.gguf",
            "ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a",
            396_705_472,
            GIB,
            2 * GIB,
            2,
            false,
            desktop_speed(1904, 2416, 909, 48.9, 777.0),
        ),
        qwen(
            "qwen3-1.7b-q4_k_m",
            "Qwen3 1.7B",
            "default",
            "available",
            "Default CPU model for local Ask SQL generation. Good balance between response speed and read-only SQLite query quality.",
            "ggml-org/Qwen3-1.7B-GGUF",
            "daeb8e2d528a760970442092f6bf1e55c3b659eb",
            "Qwen3-1.7B-Q4_K_M.gguf",
            "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5",
            1_282_439_264,
            3 * GIB,
            4 * GIB,
            4,
            false,
            desktop_speed(4138, 4611, 2352, 23.0, 296.0),
        ),
        qwen(
            "qwen3-4b-q4_k_m",
            "Qwen3 4B",
            "quality",
            "available",
            "Highest-accuracy Qwen3 CPU model for writing complex read-only SQL against the Ask projection; roughly twice as slow as 1.7B.",
            "Qwen/Qwen3-4B-GGUF",
            "bc640142c66e1fdd12af0bd68f40445458f3869b",
            "Qwen3-4B-Q4_K_M.gguf",
            "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5",
            2_497_280_256,
            5 * GIB,
            8 * GIB,
            4,
            false,
            desktop_speed(8883, 9825, 5809, 10.6, 120.0),
        ),
        qwen(
            "qwen3-8b-q4_k_m",
            "Qwen3 8B",
            "gpu",
            "gpu_future",
            "GPU tier reserved for a later catalog release. You can also load any compatible local GGUF file now via Custom model path.",
            "Qwen/Qwen3-8B-GGUF",
            "7c41481f57cb95916b40956ab2f0b139b296d974",
            "Qwen3-8B-Q4_K_M.gguf",
            "d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785",
            5_027_783_488,
            9 * GIB,
            16 * GIB,
            6,
            true,
            None,
        ),
        qwen(
            "qwen3-32b-q4_k_m",
            "Qwen3 32B",
            "gpu",
            "gpu_future",
            "Larger GPU tier reserved for a later catalog release. Listed for planning only in the downloadable catalog.",
            "Qwen/Qwen3-32B-GGUF",
            "938a7432affaec9157f883a87164e2646ae17555",
            "Qwen3-32B-Q4_K_M.gguf",
            "efd971561896866f0e910cce52761ca77b1b138090c7f15fe284676d57d1f689",
            19_762_149_024,
            24 * GIB,
            32 * GIB,
            8,
            true,
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

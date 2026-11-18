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
            context_tokens: 8192,
        },
        speed,
        backend: "llama.cpp (CPU)".to_string(),
        selectable: status == "available",
    }
}

static BASE: LazyLock<Vec<CatalogModel>> = LazyLock::new(|| {
    vec![
        qwen(
            "qwen3.5-0.8b-q4_k_m",
            "Qwen3.5 0.8B",
            "light",
            "available",
            "Fastest local SQL option for simple archive lookups. Smallest footprint, though complex multi-join queries may need SQL repair or a larger model.",
            "unsloth/Qwen3.5-0.8B-GGUF",
            "6ab461498e2023f6e3c1baea90a8f0fe38ab64d0",
            "Qwen3.5-0.8B-Q4_K_M.gguf",
            "bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517",
            532_517_120,
            GIB,
            2 * GIB,
            2,
            false,
            None,
        ),
        qwen(
            "qwen3.5-2b-q4_k_m",
            "Qwen3.5 2B",
            "default",
            "available",
            "Default CPU model for local Ask SQL generation. Good balance between response speed and read-only SQLite query quality.",
            "unsloth/Qwen3.5-2B-GGUF",
            "f6d5376be1edb4d416d56da11e5397a961aca8ae",
            "Qwen3.5-2B-Q4_K_M.gguf",
            "aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223",
            1_280_835_840,
            3 * GIB,
            4 * GIB,
            4,
            false,
            None,
        ),
        qwen(
            "qwen3.5-4b-q4_k_m",
            "Qwen3.5 4B",
            "quality",
            "available",
            "Highest-accuracy Qwen3.5 CPU model for writing complex read-only SQL against the Ask projection; roughly twice as slow as 2B.",
            "unsloth/Qwen3.5-4B-GGUF",
            "e87f176479d0855a907a41277aca2f8ee7a09523",
            "Qwen3.5-4B-Q4_K_M.gguf",
            "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
            2_740_937_888,
            5 * GIB,
            8 * GIB,
            4,
            false,
            None,
        ),
        qwen(
            "qwen3.5-9b-q4_k_m",
            "Qwen3.5 9B",
            "gpu",
            "gpu_future",
            "GPU tier reserved for a later catalog release. You can also load any compatible local GGUF file now via Custom model path.",
            "unsloth/Qwen3.5-9B-GGUF",
            "3885219b6810b007914f3a7950a8d1b469d598a5",
            "Qwen3.5-9B-Q4_K_M.gguf",
            "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8",
            5_680_522_464,
            9 * GIB,
            16 * GIB,
            6,
            true,
            None,
        ),
        qwen(
            "qwen3.5-27b-q4_k_m",
            "Qwen3.5 27B",
            "gpu",
            "gpu_future",
            "Larger GPU tier reserved for a later catalog release. Listed for planning only in the downloadable catalog.",
            "unsloth/Qwen3.5-27B-GGUF",
            "3221f178a6b842d04f1fb42f1c413534adcc0a6a",
            "Qwen3.5-27B-Q4_K_M.gguf",
            "84b5f7f112156d63836a01a69dc3f11a6ba63b10a23b8ca7a7efaf52d5a2d806",
            16_740_812_704,
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

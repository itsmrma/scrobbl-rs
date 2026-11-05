# Spike #172 run — windows-desktop-4b-nothink

- generated at (unix): 1793891663
- host: windows / x86_64 / AMD64 Family 25 Model 33 Stepping 2, AuthenticAMD / 16 logical cores / peak RSS n/a
- model: `models\Qwen3-4B-Q4_K_M.gguf` (2497280256 bytes, sha256 7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5)
- runtime: llama-cpp-2 0.1.161 / harness 0.0.0 / load 2031 ms / context 84 ms / n_ctx 2048
- config: chat `qwen3-nothink`, threads 8, max_tokens 128, warmup 3, grammar sha256 5fb807f1f63c9669a3b175421f447ae2bd59c8ee811e3781d9a9e154370b582b, prompt sha256 ca881c8014b90d5453b4016dd6fc67ff3fb911674badccd6a5decca5fe418525

| language | cases | JSON valid | schema valid | exact match | e2e p50 (ms) | e2e p95 (ms) | first token p50 (ms) | first token p95 (ms) | prefill tok/s p50 | decode tok/s p50 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| all | 30 | 30/30 | 30/30 | 30/30 | 8883 | 9825 | 5809 | 5993 | 119.6 | 10.6 |
| en | 15 | 15/15 | 15/15 | 15/15 | 8850 | 10262 | 5798 | 5957 | 119.6 | 10.5 |
| it | 15 | 15/15 | 15/15 | 15/15 | 9231 | 9825 | 5813 | 6057 | 120.0 | 10.6 |

## Failures (0)

None.

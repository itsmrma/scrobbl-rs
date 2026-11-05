# Spike #172 run — windows-desktop-qwen25-1.5b

- generated at (unix): 1793891864
- host: windows / x86_64 / AMD64 Family 25 Model 33 Stepping 2, AuthenticAMD / 16 logical cores / peak RSS n/a
- model: `models\qwen2.5-1.5b-instruct-q4_k_m.gguf` (1117320736 bytes, sha256 6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e)
- runtime: llama-cpp-2 0.1.161 / harness 0.0.0 / load 933 ms / context 12 ms / n_ctx 2048
- config: chat `qwen25`, threads 8, max_tokens 128, warmup 3, grammar sha256 5fb807f1f63c9669a3b175421f447ae2bd59c8ee811e3781d9a9e154370b582b, prompt sha256 ca881c8014b90d5453b4016dd6fc67ff3fb911674badccd6a5decca5fe418525

| language | cases | JSON valid | schema valid | exact match | e2e p50 (ms) | e2e p95 (ms) | first token p50 (ms) | first token p95 (ms) | prefill tok/s p50 | decode tok/s p50 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| all | 30 | 30/30 | 30/30 | 29/30 | 3860 | 4941 | 2148 | 2309 | 322.2 | 26.9 |
| en | 15 | 15/15 | 15/15 | 15/15 | 3786 | 5147 | 2152 | 2311 | 321.7 | 26.9 |
| it | 15 | 15/15 | 15/15 | 14/15 | 3869 | 4941 | 2147 | 2309 | 323.4 | 26.9 |

## Failures (1)

| id | lang | question | problem | output |
| --- | --- | --- | --- | --- |
| it-010 | it | Quand'è stata l'ultima volta che ho ascoltato l'album Homogenic? | interpretation mismatch | `{   "intent": "last_play",   "target": "album",   "name": "Homogenic",   "period": "last_week",   "weekday": "any",   "limit": "1" }` |

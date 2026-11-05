# Spike #172 run — windows-desktop-1.7b-nothink

- generated at (unix): 1793891358
- host: windows / x86_64 / AMD64 Family 25 Model 33 Stepping 2, AuthenticAMD / 16 logical cores / peak RSS n/a
- model: `models\Qwen3-1.7B-Q4_K_M.gguf` (1282439264 bytes, sha256 d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5)
- runtime: llama-cpp-2 0.1.161 / harness 0.0.0 / load 1007 ms / context 32 ms / n_ctx 2048
- config: chat `qwen3-nothink`, threads 8, max_tokens 128, warmup 3, grammar sha256 5fb807f1f63c9669a3b175421f447ae2bd59c8ee811e3781d9a9e154370b582b, prompt sha256 ca881c8014b90d5453b4016dd6fc67ff3fb911674badccd6a5decca5fe418525

| language | cases | JSON valid | schema valid | exact match | e2e p50 (ms) | e2e p95 (ms) | first token p50 (ms) | first token p95 (ms) | prefill tok/s p50 | decode tok/s p50 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| all | 30 | 30/30 | 30/30 | 28/30 | 4138 | 4611 | 2352 | 2529 | 295.5 | 23.0 |
| en | 15 | 15/15 | 15/15 | 13/15 | 4242 | 4808 | 2345 | 2529 | 296.1 | 23.0 |
| it | 15 | 15/15 | 15/15 | 15/15 | 4138 | 4611 | 2367 | 2545 | 295.2 | 23.0 |

## Failures (2)

| id | lang | question | problem | output |
| --- | --- | --- | --- | --- |
| en-006 | en | How many plays did I have on Fridays this year? | interpretation mismatch | `{"intent":"count_plays","target":"artist","name":"Fridays","period":"this_year","weekday":"friday","limit":"1"}` |
| en-010 | en | How many plays did I have today? | interpretation mismatch | `{"intent":"count_plays","target":"artist","name":"today","period":"today","weekday":"any","limit":"1"}` |

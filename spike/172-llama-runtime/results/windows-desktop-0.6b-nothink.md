# Spike #172 run — windows-desktop-0.6b-nothink

- generated at (unix): 1793891733
- host: windows / x86_64 / AMD64 Family 25 Model 33 Stepping 2, AuthenticAMD / 16 logical cores / peak RSS n/a
- model: `models\Qwen3-0.6B-Q4_K_M.gguf` (396705472 bytes, sha256 ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a)
- runtime: llama-cpp-2 0.1.161 / harness 0.0.0 / load 451 ms / context 32 ms / n_ctx 2048
- config: chat `qwen3-nothink`, threads 8, max_tokens 128, warmup 3, grammar sha256 5fb807f1f63c9669a3b175421f447ae2bd59c8ee811e3781d9a9e154370b582b, prompt sha256 ca881c8014b90d5453b4016dd6fc67ff3fb911674badccd6a5decca5fe418525

| language | cases | JSON valid | schema valid | exact match | e2e p50 (ms) | e2e p95 (ms) | first token p50 (ms) | first token p95 (ms) | prefill tok/s p50 | decode tok/s p50 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| all | 30 | 30/30 | 30/30 | 23/30 | 1904 | 2416 | 909 | 950 | 776.7 | 48.9 |
| en | 15 | 15/15 | 15/15 | 13/15 | 1904 | 2538 | 910 | 1045 | 771.8 | 49.0 |
| it | 15 | 15/15 | 15/15 | 10/15 | 2079 | 2316 | 908 | 950 | 777.9 | 48.9 |

## Failures (7)

| id | lang | question | problem | output |
| --- | --- | --- | --- | --- |
| en-003 | en | When did I last listen to Blink-182? | interpretation mismatch | `{"intent":"last_play","target":"artist","name":"Blink-182","period":"last_week","weekday":"any","limit":"1"}` |
| en-008 | en | When did I last play "Don't Stop Me Now"? | interpretation mismatch | `{"intent":"last_play","target":"track","name":"Don't Stop Me Now","period":"last_week","weekday":"any","limit":"1"}` |
| it-006 | it | Quanti ascolti ho fatto ieri? | interpretation mismatch | `{"intent":"last_play","target":"none","name":"","period":"yesterday","weekday":"any","limit":"1"}` |
| it-008 | it | Qual è il mio artista più ascoltato di lunedì quest'anno? | interpretation mismatch | `{"intent":"top","target":"artist","name":"","period":"this_week","weekday":"monday","limit":"1"}` |
| it-009 | it | Quante volte ho ascoltato Björk questo mese? | interpretation mismatch | `{"intent":"count_plays","target":"artist","name":"Björk","period":"last_month","weekday":"any","limit":"1"}` |
| it-010 | it | Quand'è stata l'ultima volta che ho ascoltato l'album Homogenic? | interpretation mismatch | `{"intent":"last_play","target":"album","name":"Homogenic","period":"last_week","weekday":"any","limit":"1"}` |
| it-011 | it | Quante volte ho ascoltato il brano Jóga quest'anno? | interpretation mismatch | `{"intent":"count_plays","target":"track","name":"Jóga","period":"last_year","weekday":"any","limit":"1"}` |

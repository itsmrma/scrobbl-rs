# Spike #172 run — windows-desktop-0.6b-nothink

- generated at (unix): 1793888540
- host: windows / x86_64 / AMD64 Family 25 Model 33 Stepping 2, AuthenticAMD / 16 logical cores / peak RSS n/a
- model: `models\Qwen3-0.6B-Q4_K_M.gguf` (396705472 bytes, sha256 ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a)
- runtime: llama-cpp-2 0.1.161 / harness 0.0.0 / load 434 ms / context 42 ms / n_ctx 2048
- config: chat `qwen3-nothink`, threads 8, max_tokens 128, warmup 3, grammar sha256 5fb807f1f63c9669a3b175421f447ae2bd59c8ee811e3781d9a9e154370b582b, prompt sha256 d7556110bedc3ee4d2531eafe3e7598dc7249950efe2b7039c361f241a8c8d1a

| language | cases | JSON valid | schema valid | exact match | e2e p50 (ms) | e2e p95 (ms) | first token p50 (ms) | first token p95 (ms) | prefill tok/s p50 | decode tok/s p50 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| all | 30 | 30/30 | 30/30 | 8/30 | 2306 | 2691 | 534 | 676 | 747.2 | 51.1 |
| en | 15 | 15/15 | 15/15 | 3/15 | 2213 | 2691 | 518 | 617 | 762.1 | 51.5 |
| it | 15 | 15/15 | 15/15 | 5/15 | 2412 | 2797 | 557 | 686 | 723.8 | 49.8 |

## Failures (22)

| id | lang | question | problem | output |
| --- | --- | --- | --- | --- |
| en-002 | en | When did I first play 坂本龍一? | interpretation mismatch | `{"intent": "first_play", "target": "artist", "name": "坂本龍一", "period": "this_week", "weekday": "any", "limit": "1"}` |
| en-003 | en | When did I last listen to Blink-182? | interpretation mismatch | `{"intent": "last_play", "target": "track", "name": "Blink-182", "period": "yesterday", "weekday": "saturday", "limit": "1"}` |
| en-004 | en | How many plays did I have last month? | interpretation mismatch | `{"intent": "last_play", "target": "track", "name": "I", "period": "last_month", "weekday": "any", "limit": "5"}` |
| en-005 | en | What was my most played artist last year? | interpretation mismatch | `{"intent": "last_play", "target": "artist", "name": "artist", "period": "last_year", "weekday": "any", "limit": "25"}` |
| en-006 | en | How many plays did I have on Fridays this year? | interpretation mismatch | `{"intent": "last_play", "target": "track", "name": "I", "period": "last_year", "weekday": "friday", "limit": "1"}` |
| en-007 | en | When did I first listen to AC/DC? | interpretation mismatch | `{"intent": "first_play", "target": "artist", "name": "AC/DC", "period": "this_week", "weekday": "wednesday", "limit": "1"}` |
| en-008 | en | When did I last play "Don't Stop Me Now"? | interpretation mismatch | `{"intent": "last_play", "target": "track", "name": "Don't Stop Me Now", "period": "last_week", "weekday": "saturday", "limit": "1"}` |
| en-009 | en | What are my top 10 artists of all time? | interpretation mismatch | `{"intent": "top", "target": "artist", "name": "artists", "period": "all_time", "weekday": "any", "limit": "10"}` |
| en-010 | en | How many plays did I have today? | interpretation mismatch | `{"intent": "count_plays", "target": "track", "name": "I", "period": "today", "weekday": "any", "limit": "1"}` |
| en-011 | en | When did I first listen to Simon & Garfunkel? | interpretation mismatch | `{"intent": "first_play", "target": "artist", "name": "Simon & Garfunkel", "period": "today", "weekday": "any", "limit": "1"}` |
| en-012 | en | What was my top album last week? | interpretation mismatch | `{"intent": "top", "target": "album", "name": "my", "period": "last_week", "weekday": "any", "limit": "25"}` |
| en-013 | en | How many times did I play Sgt. Pepper's Lonely Hearts Club Band? | interpretation mismatch | `{"intent": "count_plays", "target": "track", "name": "Sgt. Pepper's Lonely Hearts Club Band", "period": "all_time", "weekday": "any", "limit": "1"}` |
| it-002 | it | Quando ho ascoltato per la prima volta 坂本龍一? | interpretation mismatch | `{"intent": "first_play", "target": "artist", "name": "坂本龍一", "period": "today", "weekday": "any", "limit": "1"}` |
| it-003 | it | Qual è stato il mio artista più ascoltato il mese scorso? | interpretation mismatch | `{"intent": "top", "target": "artist", "name": "none", "period": "last_month", "weekday": "any", "limit": "1"}` |
| it-004 | it | Quand'è l'ultima volta che ho ascoltato Blink-182? | interpretation mismatch | `{"intent": "last_play", "target": "track", "name": "Blink-182", "period": "last_week", "weekday": "sunday", "limit": "10"}` |
| it-006 | it | Quanti ascolti ho fatto ieri? | interpretation mismatch | `{"intent": "unsupported", "target": "none", "name": "", "period": "yesterday", "weekday": "any", "limit": "1"}` |
| it-007 | it | Qual è il mio album più ascoltato di sempre? | interpretation mismatch | `{"intent": "top", "target": "album", "name": "my_album", "period": "all_time", "weekday": "any", "limit": "25"}` |
| it-008 | it | Qual è il mio artista più ascoltato di lunedì quest'anno? | interpretation mismatch | `{"intent": "top", "target": "artist", "name": "none", "period": "this_week", "weekday": "any", "limit": "1"}` |
| it-009 | it | Quante volte ho ascoltato Björk questo mese? | interpretation mismatch | `{"intent": "count_plays", "target": "artist", "name": "Björk", "period": "this_month", "weekday": "any", "limit": "10"}` |
| it-010 | it | Quand'è stata l'ultima volta che ho ascoltato l'album Homogenic? | interpretation mismatch | `{"intent": "last_play", "target": "album", "name": "Homogenic", "period": "last_week", "weekday": "sunday", "limit": "1"}` |
| it-011 | it | Quante volte ho ascoltato il brano Jóga quest'anno? | interpretation mismatch | `{"intent": "count_plays", "target": "track", "name": "Jóga", "period": "last_year", "weekday": "any", "limit": "10"}` |
| it-012 | it | Qual è il mio artista più ascoltato in assoluto? | interpretation mismatch | `{"intent": "top", "target": "artist", "name": "none", "period": "all_time", "weekday": "any", "limit": "25"}` |

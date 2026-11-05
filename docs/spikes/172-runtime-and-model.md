# Spike #172 — Runtime e modello AI sul NAS

**Stato:** harness pronto; misure da eseguire sull'hardware. Nessun codice di
produzione. Harness usa-e-getta in `spike/172-llama-runtime/`, branch
`spike/172-llama-runtime`, non in `main`.

Il ticket decide il runtime e il modello della v1 CPU-only. Non riapre le
decisioni di prodotto di #171: sceglie solo quello che le misure reali
permettono di sostenere.

## Hardware e ambienti

| ambiente | CPU | arch | RAM | note |
| --- | --- | --- | --- | --- |
| NAS TrueNAS SCALE | Intel i5-10500 (6C/12T, 3.1–4.5 GHz) | x86_64 | 32 GB | target delle misure di latenza |
| Mac | da compilare | arm64 | da compilare | build nativa macOS + Docker linux/arm64 |
| fisso | Windows + MSVC | x86_64 | da compilare | solo compile check locale, non è target |

La build locale Windows è già passata con Rust 1.99.1, CMake 4.3.1 e MSVC:
`llama-cpp-sys-2` 0.1.161 compila. Non sostituisce le build Linux/macOS, ma
esclude errori di API nel harness.

## Metodo

- `llama-cpp-2 = "=0.1.161"`, `default-features = false`; variante OpenMP
  misurata a parte.
- Contesto 2048, greedy, grammatica GBNF generata da
  `spike/172-llama-runtime/schema/ask.schema.json`.
- Schema di prova con tutte le proprietà obbligatorie; il DSL reale è di #173.
- 30 domande (15 EN, 15 IT), 3 di warm-up non conteggiate, KV cache azzerata a
  ogni caso, `reset_timings()` prima di ogni prefill.
- Metriche per caso: token di prompt, token generati, prefill ms, TTFT, decode
  ms, end-to-end ms, token/s. p50/p95 con nearest-rank. Le latenze includono
  anche i casi di rifiuto (`unsupported`).
- Ogni run salva JSON completo e tabella Markdown.

## Smoke del harness (Windows, non è un target)

Eseguito sul desktop Windows con Qwen3 0.6B Q4_K_M, 8 thread, 30 casi. Serve
solo a dimostrare che harness e grammatica funzionano end-to-end: il 0.6B non è
il default e il desktop non è il NAS.

| lingua | casi | JSON valido | schema valido | match esatto | e2e p50 (ms) | e2e p95 (ms) | TTFT p50 (ms) | prefill tok/s p50 | decode tok/s p50 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| EN | 15 | 15/15 | 15/15 | 3/15 | 2213 | 2691 | 518 | 762 | 51.5 |
| IT | 15 | 15/15 | 15/15 | 5/15 | 2412 | 2797 | 557 | 724 | 49.8 |
| tutti | 30 | 30/30 | 30/30 | 8/30 | 2306 | 2691 | 534 | 747 | 51.1 |

I 22 mismatch sono interpretazioni del 0.6B (periodi sbagliati, `track` invece
di `album`, `limit` inventato), non errori di grammatica o di schema. Il nome
Unicode `坂本龍一` e `Björk` vengono copiati correttamente da 0.6B già con questa
configurazione.

## Requisiti emersi dall'harness

Cinque comportamenti di `llama-cpp-2` 0.1.161 / llama.cpp vanno recepiti
nell'adapter di #178:

1. **`LlamaSampler::sample` accetta già il token.** Chiamare anche
   `sampler.accept(token)` fa avanzare la grammatica due volte, svuota lo stack
   e fa abortire `llama_sampler_sample` con
   `GGML_ASSERT(!stacks.empty())` (utilityai/llama-cpp-rs#1007). Il harness usa
   solo `sample`.
2. **`llama_context_default_params()` imposta `no_perf = true`.** Senza
   `.with_no_perf(false)` i tempi di decode risultano 0 e il decode tok/s non è
   misurabile.
3. **In GBNF le virgolette JSON fanno parte del literal.** Una chiave va scritta
   `"\"intent\""`, non `"intent"`; lo stesso per i valori enum. Il primo
   generatore le ometteva e la grammatica accettava JSON non valido (`intent:`
   senza virgolette).
4. **`default-features = false` è sufficiente**: la build senza `common` ha
   prodotto JSON valido; la feature `common` è rimasta solo come confronto.
5. **Gli output vanno accumulati come byte**, non decodificati token per token:
   un carattere UTF-8 può essere diviso tra due token e `from_utf8_lossy` per
   token lo corromperebbe.

## Matrice di build

| target | comando | esito | note |
| --- | --- | --- | --- |
| macOS arm64 nativo | `cargo build --release` | da eseguire | |
| Linux x86_64 (NAS) | `docker build -t scrobbl-spike:1.7b .` | da eseguire | |
| Linux arm64 | `docker build --platform linux/arm64 ...` dal Mac | da eseguire | |
| Linux x86_64 + OpenMP | `--build-arg CARGO_FEATURES=openmp` | da eseguire | confronto `libgomp` |

## Latenza

### Qwen3 1.7B Q4_K_M — default candidato

| lingua | casi | JSON valido | schema valido | match esatto | e2e p50 (ms) | e2e p95 (ms) | TTFT p50 (ms) | prefill tok/s p50 | decode tok/s p50 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| EN | 15 | | | | | | | | |
| IT | 15 | | | | | | | | |
| tutti | 30 | | | | | | | | |

Note:

### Qwen3 4B Q4_K_M

| lingua | casi | JSON valido | schema valido | match esatto | e2e p50 (ms) | e2e p95 (ms) | TTFT p50 (ms) | prefill tok/s p50 | decode tok/s p50 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| EN | 15 | | | | | | | | |
| IT | 15 | | | | | | | | |
| tutti | 30 | | | | | | | | |

Note:

### Tier opzionali (0.6B, Qwen2.5 1.5B, OpenMP)

| modello | variante | e2e p50 (ms) | e2e p95 (ms) | decode tok/s p50 | schema valido | match esatto |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Qwen3 0.6B Q4_K_M | CPU, no OpenMP | | | | | |
| Qwen2.5 1.5B Instruct Q4_K_M | CPU, no OpenMP | | | | | |
| Qwen3 1.7B Q4_K_M | CPU + OpenMP | | | | | |

## Thinking mode

La grammatica obbliga il primo token a `{`, quindi il thinking **non può
comparire** nell'output: con GBNF il confronto non dice se il thinking è
disattivabile. Per verificarlo servono due run **senza grammatica**:

1. `--chat qwen3-think --no-grammar`: cercare ` thinking` nell'output.
2. `--chat qwen3-nothink --no-grammar`: il prefisso vuoto deve impedirlo.

Se (1) produce thinking e (2) no, la disattivazione via prefisso è affidabile
per il modello. Se (1) non produce thinking nemmeno senza grammatica, la
grammatica non serve a sopprimerlo. Con grammatica si misura solo la qualità
del JSON forzato.

| run | thinking emesso | JSON valido | match esatto | note |
| --- | --- | --- | --- | --- |
| `qwen3-think --no-grammar` | | n/a | n/a | |
| `qwen3-nothink --no-grammar` | | n/a | n/a | |
| `qwen3-think` con grammatica | no, per costruzione | | | |
| `qwen3-nothink` con grammatica | no, per costruzione | | | |

Decisione: [ ] thinking disattivabile in modo affidabile con Qwen3 · [ ] serve
il fallback Qwen2.5

## Nomi Unicode e caratteri speciali

Casi dedicati: Björk, 坂本龍一, Blink-182, AC/DC, `Don't Stop Me Now`,
`Sgt. Pepper's Lonely Hearts Club Band`, Simon & Garfunkel, Sigur Rós, Jóga.

| caso | output del modello | esito |
| --- | --- | --- |
| en-001 Björk | | |
| en-002 坂本龍一 | | |
| en-008 Don't Stop Me Now | | |
| en-013 Sgt. Pepper's… | | |
| it-013 Sigur Rós | | |

Nessun pattern ASCII è applicato: lo slot `name` usa la regola `string` di
llama.cpp. Se un nome viene alterato, è un limite del modello, non del
validatore.

## Packaging e budget

| voce | valore | budget | esito |
| --- | ---: | ---: | --- |
| delta immagine vs `debian:bookworm-slim` | | ≤ 25 MiB | |
| binario harness (stripped, LTO) | | ≤ 30 MiB | |
| delta immagine con OpenMP | | ≤ 25 MiB | |
| build con root filesystem read-only | | deve riuscire | |
| modello su volume `/data/models` read-only | | deve riuscire | |
| binario Windows, release (solo sanity check) | 4.6 MiB | ≤ 30 MiB | passa fuori target |

OpenMP richiede **entrambi** gli argomenti:
`--build-arg CARGO_FEATURES=openmp --build-arg INSTALL_LIBGOMP=1`.

Il delta qui misura il **runtime del modello**, non l'immagine ufficiale
`scrobbl-rs`: la seconda va rimisurata in #180 dopo aver collegato
`llama-cpp-2` all'app. Il binario harness è una stima diretta del costo statico
di llama.cpp in questa configurazione (stripped, LTO, senza OpenMP).

## Failure modes osservati

- Nessuna build fallita su Windows/MSVC; la build Linux/macOS è ancora da
  eseguire.
- Con il primo generatore GBNF (chiavi senza virgolette) il modello produceva
  JSON non valido pur rispettando la grammatica. Corretto; il validatore
  `schema` ha intercettato il problema.
- Il mismatch di 0.6B è semantico: ad esempio `all_time` interpretato come
  `this_week`, `album` come `track`, `limit` inventato. Nessun `name` Unicode è
  stato corrotto.
- `decode_ms` restava 0 finché non è stato impostato `.with_no_perf(false)`.
- Nessun OOM con 0.6B e contesto 2048 sul desktop (RSS non disponibile su
  Windows).

## Decisione

Da compilare e riportare su #171:

- **Runtime:** `llama-cpp-2` 0.1.161, `default-features = false` + OpenMP sì/no.
- **Famiglia:** Qwen3 oppure Qwen2.5.
- **Dimensione default:** 0.6B / 1.7B / 4B.
- **Classe di latenza:** soglie p50/p95 concordate e misurate.
- **Vincoli scoperti:** RAM, disco, prima domanda a freddo, thread.

## Checklist di accettazione (#172)

- [ ] Build riuscita su macOS arm64 e Linux x86_64/arm64.
- [ ] Qwen3 1.7B carica e produce JSON valido per lo schema di prova.
- [ ] Thinking mode disattivato, oppure fallback Qwen2.5 provato e documentato.
- [ ] Nomi Unicode accettati dagli slot testuali senza pattern ASCII restrittivi.
- [ ] Latenza p50/p95 misurata sul NAS per 1.7B e 4B.
- [ ] Dimensione di binario e immagine misurata contro il budget.
- [ ] Report e harness collegati; decisione registrata su #171.

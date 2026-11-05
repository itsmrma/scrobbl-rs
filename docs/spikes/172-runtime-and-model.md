# Spike #172 — Runtime e modello AI sul NAS

**Stato:** fase A (desktop) completata. Fase B (NAS e Mac) rimandata a dopo
l'integrazione UI, insieme a #180/#182. Nessun codice di produzione. Harness
usa-e-getta in `spike/172-llama-runtime/`, branch `spike/172-llama-runtime`, non
in `main`.

Il ticket decide il runtime e il modello della v1 CPU-only. Non riapre le
decisioni di prodotto di #171: sceglie solo quello che le misure reali
permettono di sostenere.

## Hardware e ambienti

| ambiente | CPU | arch | RAM | note |
| --- | --- | --- | --- | --- |
| fisso Windows | AMD Ryzen 7 5700X (8C/16T, 3.4 GHz) | x86_64 | 24 GB | fase A: matrice modelli |
| NAS TrueNAS SCALE | Intel i5-10500 (6C/12T, 3.1–4.5 GHz) | x86_64 | 32 GB | fase B: latenza e packaging |
| Mac | da compilare | arm64 | da compilare | fase B: build nativa macOS + Docker arm64 |

La build Windows è passata con Rust 1.99.1, CMake 4.3.1 e MSVC:
`llama-cpp-sys-2` 0.1.161 compila. Non sostituisce le build Linux/macOS, ma
esclude errori di API nel harness.

## Sequenza delle misure

- **Fase A — desktop:** matrice completa su Windows nativo. Produce la decisione
  su runtime, famiglia, dimensione default e classe di latenza provvisoria.
  Docker/WSL non servono.
- **Fase B — CI e NAS, dopo l'integrazione UI:** build Linux x86_64/arm64 e
  macOS arm64 su GitHub Actions (`spike-172.yml`), immagine Docker amd64/arm64,
  rootfs read-only + `/data` sul NAS, latenza definitiva e budget immagine
  sull'immagine ufficiale (#180/#182). Il NAS non compila: carica l'artifact
  `amd64` prodotto dalla CI.

## Metodo

- `llama-cpp-2 = "=0.1.161"`, `default-features = false`; variante OpenMP da
  misurare in fase B.
- Contesto 2048, greedy, grammatica GBNF generata da
  `spike/172-llama-runtime/schema/ask.schema.json`.
- Schema di prova con tutte le proprietà obbligatorie; il DSL reale è di #173.
- Prompt compatto in inglese con quattro esempi (EN/IT), mappatura esplicita dei
  periodi e regola "most played = limit 1". `target: none` accetta qualunque
  target con `name` vuoto, perché lo schema di prova richiede un campo che il
  DSL reale avrebbe opzionale.
- 30 domande (15 EN, 15 IT), 3 di warm-up non conteggiate, KV cache azzerata a
  ogni caso, `reset_timings()` prima di ogni prefill.
- Metriche per caso: token di prompt, token generati, prefill ms, TTFT, decode
  ms, end-to-end ms, token/s. p50/p95 con nearest-rank. Le latenze includono
  anche i casi di rifiuto (`unsupported`).
- Ogni run salva JSON completo e tabella Markdown; il riepilogo è in
  `spike/172-llama-runtime/results/windows-desktop-summary.md`.

## Fase A — risultati desktop

4 modelli, 30 casi, 8 thread. Il desktop non è il target: i numeri servono a
scegliere la famiglia e la dimensione, non a fissare la latenza del NAS.

| modello | lingua | casi | schema valido | match esatto | e2e p50 (ms) | e2e p95 (ms) | TTFT p50 (ms) | prefill tok/s p50 | decode tok/s p50 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Qwen3 1.7B Q4_K_M | EN | 15 | 15/15 | 13/15 | 4242 | 4808 | 2345 | 296 | 23.0 |
| Qwen3 1.7B Q4_K_M | IT | 15 | 15/15 | 15/15 | 4138 | 4611 | 2367 | 295 | 23.0 |
| Qwen3 1.7B Q4_K_M | tutti | 30 | 30/30 | 28/30 | 4138 | 4611 | 2352 | 296 | 23.0 |
| Qwen3 4B Q4_K_M | EN | 15 | 15/15 | 15/15 | 8850 | 10262 | 5798 | 120 | 10.5 |
| Qwen3 4B Q4_K_M | IT | 15 | 15/15 | 15/15 | 9231 | 9825 | 5813 | 120 | 10.6 |
| Qwen3 4B Q4_K_M | tutti | 30 | 30/30 | 30/30 | 8883 | 9825 | 5809 | 120 | 10.6 |
| Qwen3 0.6B Q4_K_M | EN | 15 | 15/15 | 13/15 | 1904 | 2538 | 910 | 772 | 49.0 |
| Qwen3 0.6B Q4_K_M | IT | 15 | 15/15 | 10/15 | 2079 | 2316 | 908 | 778 | 48.9 |
| Qwen3 0.6B Q4_K_M | tutti | 30 | 30/30 | 23/30 | 1904 | 2416 | 909 | 777 | 48.9 |
| Qwen2.5 1.5B Instruct Q4_K_M | EN | 15 | 15/15 | 15/15 | 3786 | 5147 | 2152 | 322 | 26.9 |
| Qwen2.5 1.5B Instruct Q4_K_M | IT | 15 | 15/15 | 14/15 | 3869 | 4941 | 2147 | 323 | 26.9 |
| Qwen2.5 1.5B Instruct Q4_K_M | tutti | 30 | 30/30 | 29/30 | 3860 | 4941 | 2148 | 322 | 26.9 |

Caricamento modello: 1.0 s (1.7B), 2.0 s (4B), 0.45 s (0.6B), 0.93 s (Qwen2.5
1.5B). Contesto creato in 11–87 ms.

Lettura:

- **Qwen3 4B** è l'unico 30/30, ma costa 2,3× il tempo di 1.7B e ha TTFT
  5,8 s: adatto a un tier "quality", non al default su CPU-only.
- **Qwen3 1.7B** è il default della famiglia scelta: 28/30, p50 4,1 s, 23 tok/s.
- **Qwen3 0.6B** è il tier light: 23/30 a 1,9 s. Utile solo per risposte rapide,
  non per la qualità del parser.
- **Qwen2.5 1.5B** è stato misurato come confronto (29/30, p50 3,9 s): resta
  documentato, ma la decisione di prodotto è **solo famiglia Qwen3**, quindi non
  entra nel catalogo v1.

## Thinking mode

La grammatica obbliga il primo token a `{`, quindi il thinking **non può
comparire** nell'output: con GBNF il confronto non dice se il thinking è
disattivabile. I due run **senza grammatica** rispondono alla domanda:

| run | thinking emesso | JSON valido | match esatto | note |
| --- | ---: | ---: | ---: | --- |
| `qwen3-think --no-grammar` | 30/30 | 0/30 | 0/30 | il prefisso aperto fa partire ` thinking` |
| `qwen3-nothink --no-grammar` | 0/30 | 28/30 | 26/30 | il blocco vuoto lo sopprime |
| `qwen3-think` con grammatica | 0/30 per costruzione | 30/30 | 28/30 | la GBNF forza `{` |
| `qwen3-nothink` con grammatica | 0/30 per costruzione | 30/30 | 28/30 | default misurato |

Conclusione: **il thinking mode di Qwen3 è disattivabile in modo affidabile**
con il prefisso `assistant\n<think>\n\n</think>\n\n`. La famiglia resta Qwen3;
Qwen2.5 è stato misurato solo come confronto e non entra nel catalogo v1.

## Nomi Unicode e caratteri speciali

Casi dedicati: Björk, 坂本龍一, Blink-182, AC/DC, `Don't Stop Me Now`,
`Sgt. Pepper's Lonely Hearts Club Band`, Simon & Garfunkel, Sigur Rós, Jóga.

| caso | esito |
| --- | --- |
| en-001 Björk | nome corretto su tutti i modelli |
| en-002 坂本龍一 | nome corretto; 0.6B sbaglia solo il periodo |
| en-008 Don't Stop Me Now | nome corretto; alcuni modelli sbagliano il periodo |
| en-013 Sgt. Pepper's… | nome corretto; target album corretto su 1.7B/4B/Qwen2.5 |
| it-013 Sigur Rós | nome e target corretti su tutti i modelli |

Nessun pattern ASCII è applicato: lo slot `name` usa la regola `string` di
llama.cpp. I mismatch residui sono semantici (periodo o entità), non di
codifica.

## Failure modes osservati

- **Qwen3 4B:** 0 mismatch.
- **Qwen2.5 1.5B:** un solo mismatch, `it-010` ("l'ultima volta… Homogenic")
  interpretato come `last_week` invece di `all_time`.
- **Qwen3 1.7B:** due mismatch, `en-006` e `en-010`, dove "Fridays" e "today"
  finiscono in `name` invece che in `weekday`/niente.
- **Qwen3 0.6B:** sette mismatch, soprattutto mappatura dei periodi
  (`last_play` → `last_week`, `quest'anno` → `this_week`/`last_year`) e
  `it-006` classificato `last_play` invece di `count_plays`.
- **Senza grammatica:** 27–28/30 JSON validi; un'injection (`it-015`) ha
  prodotto SQL, quindi la GBNF è necessaria anche come barriera di formato.
- **Grammatica:** con GBNF tutti i 4 modelli hanno prodotto 30/30 JSON validi.
- La prima versione del generatore GBNF (chiavi senza virgolette) produceva JSON
  non valido pur rispettando la grammatica; il validatore `schema` l'ha
  intercettato.
- `decode_ms` restava 0 finché non è stato impostato `.with_no_perf(false)`.

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
   `"\"intent\""`, non `"intent"`; lo stesso per i valori enum.
4. **`default-features = false` è sufficiente**: la build senza `common` ha
   prodotto JSON valido; la feature `common` è rimasta solo come confronto.
5. **Gli output vanno accumulati come byte**, non decodificati token per token:
   un carattere UTF-8 può essere diviso tra due token e `from_utf8_lossy` per
   token lo corromperebbe.

## Matrice di build

Le build di fase B girano su GitHub Actions nel workflow `spike-172.yml`, non sul
NAS né sul Mac: ogni job produce binario, `size.txt` e immagine Docker.

| target | job | esito | note |
| --- | --- | --- | --- |
| Windows x86_64 (fase A) | locale | fatto | 4 modelli, 30/30 JSON validi ciascuno |
| macOS arm64 | `Native macos-arm64` | da eseguire | runner `macos-14` |
| Linux x86_64 | `Native linux-amd64` | da eseguire | runner `ubuntu-latest` |
| Linux arm64 | `Native linux-arm64` | da eseguire | runner `ubuntu-24.04-arm` |
| Docker linux/amd64 | `Docker linux/amd64` | da eseguire | artifact `spike-172-docker-amd64` |
| Docker linux/arm64 | `Docker linux/arm64` | da eseguire | artifact `spike-172-docker-arm64` |

Il workflow parte su push del branch `spike/172-llama-runtime` o manualmente da
`workflow_dispatch`. Non carica modelli e non esegue inferenza.

## Packaging e budget

| voce | valore | budget | esito |
| --- | ---: | ---: | --- |
| binario Windows, release (stripped, LTO) | 4,4 MiB | ≤ 30 MiB | passa fuori target |
| delta immagine vs `debian:bookworm-slim` | da misurare in fase B | ≤ 25 MiB | |
| delta immagine con OpenMP | da misurare in fase B | ≤ 25 MiB | |
| build con root filesystem read-only | da eseguire in fase B | deve riuscire | |
| modello su volume `/data/models` read-only | da eseguire in fase B | deve riuscire | |

OpenMP richiede **entrambi** gli argomenti:
`--build-arg CARGO_FEATURES=openmp --build-arg INSTALL_LIBGOMP=1`.

Il delta immagine va misurato sull'immagine ufficiale `scrobbl-rs` in #180, dopo
aver collegato `llama-cpp-2` all'app. Il binario harness (4,4 MiB su Windows) è
una stima diretta del costo statico di llama.cpp in questa configurazione.

## Decisione

Famiglia decisa: **solo Qwen3**. Da registrare su #171.

- **Runtime:** `llama-cpp-2` 0.1.161 con `default-features = false`; la feature
  `common` non è necessaria. OpenMP e packaging restano fase B.
- **Default:** Qwen3 1.7B Q4_K_M — 28/30, p50 4,1 s, p95 4,6 s, TTFT 2,35 s,
  23 tok/s sul desktop.
- **Light:** Qwen3 0.6B Q4_K_M — 23/30, p50 1,9 s, 49 tok/s.
- **Quality:** Qwen3 4B Q4_K_M — 30/30, ma p50 8,9 s e TTFT 5,8 s; tier
  selezionabile, non default CPU-only.
- **Thinking:** disattivato in modo affidabile dal prefisso vuoto
  (`qwen3-think` senza grammatica: 30/30 ` thinking`; `qwen3-nothink`: 0/30).
- **Classe di latenza provvisoria (desktop):** default p50 ~4 s / p95 ~4,6 s,
  TTFT ~2,4 s. Sul NAS ci si aspetta un peggioramento; la classe definitiva si
  fissa in fase B.
- **Qwen2.5 1.5B:** misurato come confronto (29/30, p50 3,9 s) ma escluso dalla
  decisione di famiglia; resta nel report come alternativa documentata.

Vincoli: contesto 2048 sufficiente; nessun OOM con 24 GB; un modello alla volta.

## Checklist di accettazione (#172)

Fase A:

- [x] Qwen3 1.7B carica e produce JSON valido per lo schema di prova.
- [x] Thinking mode disattivato in modo affidabile; fallback Qwen2.5 misurato.
- [x] Nomi Unicode accettati dagli slot testuali senza pattern ASCII restrittivi.
- [x] Latenza p50/p95 misurata sul desktop per 1.7B e 4B.
- [x] Dimensione del binario misurata (4,4 MiB su Windows).
- [ ] Decisione registrata su #171.

Fase B, dopo l'integrazione UI:

- [ ] Build riuscita su macOS arm64 e Linux x86_64/arm64 via GitHub Actions.
- [ ] Latenza p50/p95 confermata sul NAS per 1.7B e 4B.
- [ ] Dimensione immagine misurata contro il budget sull'immagine ufficiale.
- [ ] Rootfs read-only e volume `/data` verificati.

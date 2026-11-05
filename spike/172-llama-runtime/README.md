# Spike #172 — runtime e modello AI sul NAS

Harness usa-e-getta per verificare `llama-cpp-2` + Qwen3/Qwen2.5 su NAS Linux
x86_64 e macOS arm64. **Non è codice di produzione e non va in `main`.** Il
report vive in [`docs/spikes/172-runtime-and-model.md`](../../docs/spikes/172-runtime-and-model.md).

## Cosa misura

- build di `llama-cpp-2` **0.1.161** pinnato, `default-features = false`;
- caricamento di un GGUF con contesto 2k;
- grammatica GBNF generata da `schema/ask.schema.json`;
- JSON valido e interpretazione corretta su 30 domande reali (15 EN, 15 IT),
  incluse Björk, 坂本龍一, Blink-182, `Don't Stop Me Now`, `Sigur Rós`, AC/DC;
- prefill/decode token/s, TTFT ed end-to-end p50/p95;
- thinking mode Qwen3 (`qwen3-nothink` vs `qwen3-think`) e fallback Qwen2.5;
- dimensione del binario e delta dell'immagine rispetto al budget.

Il corpus e lo schema sono **di prova**: il DSL vero arriva con #173 e
l'adapter con #178. Qui servono solo a mettere sotto stress runtime e modello.

## Sequenza delle misure

1. **Fase desktop (questa):** matrice completa su Windows/Linux nativo. Decide
   runtime, famiglia di modelli, dimensione default e classe di latenza
   provvisoria.
2. **Fase NAS/Mac (dopo l'integrazione UI):** build Linux/arm64, immagine
   Docker, read-only + `/data` e conferma della latenza sul target, insieme a
   #182. In questa fase si rifà anche il budget immagine sull'immagine
   ufficiale (#180).

Docker e WSL non sono richiesti per la fase desktop: bastano Rust, CMake e un
toolchain C++.

La fase A è già stata eseguita sul desktop Ryzen 7 5700X: risultati e decisione
provvisoria sono in
[`docs/spikes/172-runtime-and-model.md`](../../docs/spikes/172-runtime-and-model.md).

## Prerequisiti

- Rust 1.99.1 (il repo usa questa versione) e toolchain C++.
- `cmake`.
- Linux: `clang` + `libclang-dev` (bindgen), `libstdc++6` a runtime.
- macOS arm64: Xcode Command Line Tools e `cmake` (`brew install cmake`).
- Docker su NAS e, per `linux/arm64`, Docker Desktop sul Mac.

### Trasferire l'harness sul NAS (fase finale)

TrueNAS SCALE non è un ambiente di sviluppo: copia solo questa directory (senza
`models/` e `target/`) su un dataset, per esempio:

```bash
scp -r spike/172-llama-runtime admin@nas:/mnt/tank/apps/scrobbl-spike/harness
```

Poi entra in una shell root (`sudo -i`) perché i comandi Docker di TrueNAS
girano come root. Modelli e risultati stanno sullo stesso dataset:

```bash
cd /mnt/tank/apps/scrobbl-spike/harness
python3 scripts/fetch-model.py --id qwen3-1.7b-q4_k_m --dir models
docker build -t scrobbl-spike:1.7b .
```

## 1. Scaricare i modelli

Da questa directory:

```bash
python3 scripts/fetch-model.py --list
python3 scripts/fetch-model.py --id qwen3-1.7b-q4_k_m --dir models
python3 scripts/fetch-model.py --id qwen3-4b-q4_k_m --dir models
```

Gli SHA-256 e le revisioni Hugging Face sono pinnati in `models.json`; lo
script riprende con `Range`, scrive `.part`, verifica lo SHA-256 e rinomina solo
alla fine. Il modello 1.7B è ~1,2 GiB, il 4B ~2,3 GiB. `models/` è ignorata da
git.

## 2. Build nativa (macOS arm64 e Linux)

```bash
cargo build --release
```

La build Linux è quella che conta per il packaging; quella macOS serve anche a
verificare che il crate compili su arm64.

## 3. Esecuzione

```bash
cargo run --release -- \
  --model models/Qwen3-1.7B-Q4_K_M.gguf \
  --out results/nas-1.7b-nothink.json \
  --md-out results/nas-1.7b-nothink.md \
  --label nas-i5-10500-1.7b-nothink \
  --chat qwen3-nothink \
  --threads 6
```

`--threads` default a `logical cores / 2` (sull'i5-10500: 6). Il primo caso è
preceduto da `--warmup 3` (default) non conteggiato.

Per diagnosticare un blocco: `SCROBBL_SPIKE_TRACE=1` stampa la fase corrente e
`SCROBBL_SPIKE_LLAMA_LOGS=1` riabilita i log di llama.cpp. `--no-grammar` è una
sola diagnostica: misura il modello senza GBNF e non va usato per il report.
`--features common` compila la variante con il wrapper `common` di
`llama-cpp-2`; il default dello spike resta `default-features = false`.

Matrice minima da eseguire sul NAS:

| run | modello | chat | scopo |
| --- | --- | --- | --- |
| 1 | Qwen3 1.7B Q4_K_M | `qwen3-nothink` | default candidato |
| 2 | Qwen3 4B Q4_K_M | `qwen3-nothink` | qualità vs latenza |
| 3 | Qwen3 1.7B Q4_K_M | `qwen3-think --no-grammar` | thinking emesso? |
| 3b | Qwen3 1.7B Q4_K_M | `qwen3-nothink --no-grammar` | thinking soppresso? |
| 4 | Qwen3 0.6B Q4_K_M | `qwen3-nothink` | tier light |
| 5 | Qwen2.5 1.5B Instruct Q4_K_M | `qwen25` | fallback non-thinking |

I run 3/3b sono **senza grammatica** di proposito: con GBNF il primo token è
obbligato a `{`, quindi il thinking non può comparire in nessun caso e il
confronto non direbbe nulla. Cerca ` thinking` nell'output del run 3: se manca
anche lì, il modello non sta pensando; se compare nel 3 ma non nel 3b, il
prefisso vuoto del non-thinking funziona.

La matrice si lancia anche in blocco:

```bash
python3 scripts/run-matrix.py --threads 8 --label windows-desktop
```

Salta i modelli non ancora scaricati e scrive `results/<label>-<run>.json|md`.
Per il riepilogo e l'elenco dei mismatch:

```bash
python3 scripts/summarize.py results/<label>-*.json \
  --output results/<label>-summary.md
python3 scripts/analyze-results.py results/<label>-*.json \
  --output results/<label>-failures.md
```

Se il run 3 produce sempre JSON valido quanto il run 1, la grammatica sta
sopprimendo l'output di thinking; se la qualità peggiora, il default resta
`qwen3-nothink` e il fallback Qwen2.5 va provato. Registrare entrambi.

Ogni run scrive:

- `results/*.json`: dati completi per caso (token, tempi, output, errori);
- `results/*.md`: tabella pronta da incollare nel report.

## 4. Docker (NAS e linux/arm64)

```bash
# linux/amd64 sul NAS
docker build -t scrobbl-spike:1.7b .

# linux/arm64 nativo dal Mac (per l'acceptance Linux arm64)
docker build --platform linux/arm64 -t scrobbl-spike:arm64 .
```

Esecuzione con root filesystem read-only e modello montato in sola lettura:

```bash
docker run --rm --read-only \
  --tmpfs /tmp:rw,noexec,nosuid,size=64m \
  -v "$PWD/models:/data/models:ro" \
  -v "$PWD/results:/data/results" \
  scrobbl-spike:1.7b \
  --model /data/models/Qwen3-1.7B-Q4_K_M.gguf \
  --out /data/results/nas-docker-1.7b.json \
  --md-out /data/results/nas-docker-1.7b.md \
  --label nas-docker-1.7b \
  --threads 6
```

Variante OpenMP (per capire se `libgomp` serve):

```bash
docker build --build-arg CARGO_FEATURES=openmp \
  --build-arg INSTALL_LIBGOMP=1 -t scrobbl-spike:1.7b-openmp .
```

Il container gira come uid 10001: se `results/` è appena stato creato da root,
il run non può scrivere. Sull'host:

```bash
install -d -o 10001 -g 10001 results
```

In alternativa passa `--user "$(id -u):$(id -g)"` a `docker run`, ma i file
risulteranno di proprietà dell'utente host.

## 5. Budget di distribuzione

```bash
python3 scripts/measure-size.py \
  --baseline debian:bookworm-slim \
  --spike scrobbl-spike:1.7b \
  --json-out results/size-1.7b.json
```

Il budget è verificato in MiB: **delta immagine ≤ 25 MiB**, **binario ≤ 30 MiB**.
Qui il baseline è `debian:bookworm-slim` perché questa immagine contiene solo il
runtime del modello. #180 dovrà ripetere la misura sull'immagine ufficiale
`scrobbl-rs` una volta collegato il runtime all'app.

Per estrarre il binario dall'immagine e misurarlo a parte:

```bash
docker create --name spike-size scrobbl-spike:1.7b
docker cp spike-size:/usr/local/bin/scrobbl-ask-spike ./scrobbl-ask-spike.size
docker rm -f spike-size
ls -l scrobbl-ask-spike.size
```

## 6. Cosa riportare

Per ogni run: `results/*.json` + `results/*.md`, più:

- `docker image inspect --format '{{.Size}}'` dell'immagine;
- `rustc -V`, `cmake --version`, `uname -a` (o `sw_vers` su macOS);
- eventuali log di build falliti (soprattutto `libclang`/`cmake`).

Riempire le tabelle in `docs/spikes/172-runtime-and-model.md`, registrare la
decisione su #171 e chiudere #172.

## Limiti noti dello spike

- Schema di prova con **tutte le proprietà obbligatorie**: il generatore GBNF
  rifiuta le opzionali. La gestione vera è di #173/#178.
- Un solo contesto, una sola richiesta alla volta: niente coda, timeout o
  annullamento (sono di #178).
- Il modello reale non serve per i test unitari: `cargo test` copre grammatica,
  validatore, corpus e percentili senza caricare pesi.
- `llama-cpp-2` 0.1.161 non espone `chat_template_kwargs`; il prefisso ChatML è
  scritto a mano, incluso il blocco ` thinking` vuoto per il non-thinking.
- In questa versione di llama.cpp `LlamaSampler::sample` **accetta già** il
  token: non chiamare `accept` una seconda volta. Il dettaglio è in
  `docs/spikes/172-runtime-and-model.md`.

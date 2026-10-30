# Gestione della cache di compilazione e degli artefatti

Questa guida descrive la configurazione dei profili Cargo, il riuso della cache di compilazione in sviluppo, test e Docker, e la gestione dello spazio occupato dagli artefatti in `scrobbl-rs`.

## Profilo di sviluppo (`dev`) e test

In `Cargo.toml` è configurato esplicitamente il profilo `dev`:

```toml
[profile.dev]
debug = 0
incremental = true
```

### Perché `debug = 0`

- Per impostazione predefinita, Cargo genera tabelle di simboli di debug complete (`debug = 2` / DWARF / PDB / dSYM). Nei progetti con ampie dipendenze asincrone e crittografiche (Tokio, Axum, SQLx, Ring), questo genera decine di migliaia di file oggetto intermedi (`.o`) e binari di test di grandi dimensioni (da 20 a 50 MiB ciascuno), rallentando notevolmente il linking e saturando lo spazio su disco.
- Con `debug = 0` (simboli disattivati), la compilazione e il linking sono significativamente più veloci e la dimensione degli artefatti in `target/debug` si riduce di circa l'80–90%.
- `incremental = true` garantisce la conservazione dello storico di compilazione incrementale, permettendo a Cargo e `rustc` di ricompilare solo le funzioni e i moduli modificati.

### Ereditarietà nei test (`profile.test`)

Secondo le regole di Cargo, il profilo `test` eredita direttamente dal profilo `dev`. Di conseguenza:
- `cargo test` eredita automaticamente `debug = 0` e `incremental = true` senza richiedere duplicazioni nel manifest.
- I binari di test beneficiano della stessa rapidità di linking e del ridotto ingombro su disco.

### Come riabilitare temporaneamente le informazioni di debug

Quando è necessario analizzare un crash o utilizzare un debugger (LLDB/GDB) con stack trace completi e simboli:

1. **Tramite variabili d'ambiente di Cargo (consigliato, senza modificare i file):**
   ```sh
   # Per build e run in dev:
   CARGO_PROFILE_DEV_DEBUG=2 cargo build --locked

   # Per l'esecuzione dei test:
   CARGO_PROFILE_TEST_DEBUG=2 cargo test --locked --test archive
   ```
2. **Tramite flag rustc:**
   ```sh
   RUSTFLAGS="-C debuginfo=2" cargo test --locked
   ```
3. **Tramite opzione `--config` di Cargo:**
   ```sh
   cargo build --locked --config 'profile.dev.debug=2'
   ```

> [!WARNING]
> La modifica delle impostazioni del profilo o dei flag di debug altera l'impronta (fingerprint) del compilatore per i crate interessati. Il passaggio tra `debug = 0` e `debug = 2` richiederà a Cargo di ricompilare i crate coinvolti con i nuovi flag.

### Profilo di rilascio (`release`)

Il profilo `release` mantiene le ottimizzazioni predefinite standard (`opt-level = 3`, `debug = 0`, linking LTO/strip standard), garantendo massime prestazioni in produzione.

---

## Cache persistente e `CARGO_TARGET_DIR`

I controlli locali, lo sviluppo e le suite di test riutilizzano gli artefatti compatibili conservati nella directory di destinazione.

Tutti gli script di supporto e le suite di test rispettano la variabile d'ambiente `CARGO_TARGET_DIR`:
- `scripts/e2e-server.cjs` (determina il percorso del binario e invoca `cargo build`)
- `scripts/package-distribution.py` (determina la cartella di output predefinita della distribuzione)
- `scripts/verify-spotify-archive.py` (determina il binario di default per le verifiche)
- `tests/*.spec.cjs` (risolvono l'eseguibile `scrobbl-rs` rispettando `CARGO_TARGET_DIR`)

Se `CARGO_TARGET_DIR` non è definita, il valore predefinito è la cartella `target/` alla radice del repository.

### Buone pratiche

- **Non eseguire `cargo clean` automatici**: pulire la cache cancella le dipendenze compilate e costringe a riscaricare e ricompilare tutto da zero.
- **Evitare cartelle target temporanee usa-e-getta**: creare un target isolato per ogni comando duplica spazio e vanifica la cache.
- **Mantenere separati dati applicativi e artefatti di compilazione**: `SCROBBL_DATA_DIR` (il database e i file temporanei dei test) vive in percorsi isolati (`tmpdir`) ed è ripulita al termine dei test, mentre `CARGO_TARGET_DIR` rimane intatta e persistente.

---

## Avvio dei test E2E (`scripts/e2e-server.cjs`)

Nelle verifiche end-to-end, la sola presenza di un file eseguibile in `target/debug/scrobbl-rs` non garantisce che esso corrisponda ai sorgenti correnti.

`scripts/e2e-server.cjs`:
1. Invoca `cargo build --locked --bin scrobbl-rs`.
2. Lascia a Cargo la verifica di freschezza degli artefatti:
   - Se i sorgenti non sono stati modificati, Cargo conclude il controllo in frazioni di secondo (~0,1–0,2 s) e non ricompila nulla.
   - Se un sorgente è cambiato, Cargo ricompila solo il codice modificato in modo incrementale prima dell'avvio.
3. Avvia direttamente il binario risultante dal percorso `CARGO_TARGET_DIR`, garantendo gestione corretta dei segnali (`SIGTERM`/`SIGINT`) e terminazione pulita.
4. Distrugge la directory temporanea `SCROBBL_DATA_DIR` all'uscita senza intaccare la cache di compilazione Cargo.

---

## Riuso delle dipendenze in Docker (BuildKit)

Il `Dockerfile` sfrutta i mount di cache di BuildKit (`# syntax=docker/dockerfile:1`):

```dockerfile
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/work/target \
    cargo build --locked --release && \
    mkdir -p /artifacts && \
    cp /work/target/release/scrobbl-rs /artifacts/scrobbl-rs
```

### Meccanismo e garanzie

1. **Mount di cache**: Le cartelle del registro Cargo, dei repository Git e `/work/target` sono montate come cache persistente. Quando un commit modifica solo i file in `src/`, Cargo riutilizza i crate delle dipendenze già compilati, riducendo i tempi di compilazione da minuti a pochi secondi.
2. **Copia persistente prima della fine del RUN**: Poiché i mount di tipo cache esistono solo per la durata dell'istruzione `RUN`, il binario compilato viene copiato in `/artifacts/scrobbl-rs` prima del termine del comando. In questo modo lo stage `runtime` può recuperarlo con `COPY --from=build /artifacts/scrobbl-rs /usr/local/bin/scrobbl-rs`.
3. **Integrità e tracciabilità**:
   - `VCS_REF` e `SCROBBL_BUILD_REF` sono conservati fedelmente nelle etichette OCI e nel binario.
   - `cargo vendor` e `package-distribution.py` generano il bundle completo delle licenze, dei sorgenti corrispondenti e dei metadati di distribuzione.
   - I volumi runtime e i dati del database rimangono completamente immutati.

---

## Compromesso spazio/tempo e pulizia degli artefatti obsoleti

### Il compromesso della cache

- **Vantaggio**: Mantenere la cache di Cargo evita di ricompilare 200+ dipendenze e consente tempi di ciclo di test e build quasi istantanei (~0,1 s per no-op, ~1 s per modifiche incrementali all'applicazione).
- **Costo**: Cargo per sua natura accumula artefatti in `target/debug/deps` e `.fingerprint` ad ogni modifica di commit o flag del compilatore.

### Pulizia mirata degli artefatti obsoleti

Se lo spazio su disco richiede una riduzione degli artefatti accumulati nel tempo:

- **Non cancellare tutta la cartella `target`**: cancellare l'intera cartella distrugge le dipendenze compilate.
- **Rimuovere solo gli artefatti del pacchetto locale**:
  ```sh
  cargo clean -p scrobbl-rs
  ```
  Questo comando rimuove tutti i binari di test e gli oggetti intermedi storici generati per `scrobbl-rs`, liberando diversi gigabyte di spazio obsoleto, ma **conserva tutte le librerie di terze parti** compilate. La successiva ricompilazione richiederà solo pochi secondi.
- **Non toccare i dati applicativi**: `data/archive.db`, `data/private/auth.db` e i volumi Docker non devono mai essere inclusi nelle operazioni di pulizia della build.

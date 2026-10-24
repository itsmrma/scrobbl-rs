# Audit del codebase prima della release

Data: 24 ottobre 2026. Base esaminata: `85b9905` (main dopo l'integrazione della
GUI e del video). Ticket: [Audit completo del codebase e correzione dei problemi
prima della release](https://github.com/itsmrma/scrobbl-rs/issues/44).

## Metodo e perimetro

Inventario dei sorgenti tracciati, lettura dei contratti della mappa, del
glossario e dell'ADR sulla conservazione compatta; esame mirato dei percorsi di
acquisizione, consultazione, autenticazione, conservazione e distribuzione.
Ricerca di scansioni e allocazioni proporzionali allo storico, duplicazioni,
dipendenze inutilizzate, residui dopo interruzioni e discrepanze fra codice e
documentazione. I sospetti sono stati confrontati con chiamanti, schema, test e,
per le modifiche alla pulizia, riproduzioni eseguibili su directory temporanee.

L'audit copre tutte le aree nella tabella; non è una prova formale dell'assenza
di bug. Non sono stati usati archivi personali, credenziali Spotify o il server
privato. I file generati dai test browser non fanno parte della modifica.

| Area esaminata | Percorsi e verifiche |
| --- | --- |
| Avvio, configurazione e accesso | `main.rs`, `lib.rs`, `config.rs`, `auth.rs`, `account.rs`, `error.rs`, routing e middleware in `web.rs`: lock di istanza, Host/Origin, sessione/CSRF, password, arresto e riavvio. |
| Archivio e acquisizioni | `storage.rs`, `db.rs`, `ingest.rs`, `jobs.rs`, `imports.rs`, `import_worker.rs`, `spotify.rs`, `listenbrainz.rs`: transazioni, checkpoint, backpressure, limiti, riconciliazione, rate limit e cancellazione. |
| Consultazione e libreria | `calendar.rs`, `preferences.rs`, `consultation.rs`, `consultation/selection.rs`, `browse.rs`, `listening_patterns.rs`, `library.rs`, `name_groups.rs`, `review.rs`, `review/automatic.rs`, `artwork.rs`, `activities.rs` e i tre file SQL delle proiezioni: selezioni condivise, paginazione, gruppi, correzioni, Undo, cache e immagini persistenti. |
| API, export e recupero | `read_api.rs`, `api_keys.rs`, `exports.rs`, `recovery.rs`, `recovery_http.rs`, `retention.rs`: copie coerenti, cursori, revoca, provenienza, backup, ripristino isolato e rilascio dei file. |
| Schemi e contratti | `migrations/`, `release-migrations/`, `private-migrations/`, `dto.rs`, `frontend/src/generated.ts`: separazione prototipo/release/accesso, indici e vincoli, migrazioni esistenti immutate, generazione dei contratti. |
| Frontend | Inventario di `frontend/src/`; esame di navigazione, cache letture, polling, caricamento immagini, selezioni, grafici/collage, listener e cleanup; controlli TypeScript, build e suite browser sulle pagine e sui form. CSS e asset verificati anche attraverso i test responsive esistenti. |
| Test e strumenti | `tests/`, fixture, helper CJS, `scripts/`, Playwright: isolamento dei dati, copertura di crash/ripresa/recupero, import reali da fixture e carichi sintetici; helper di navigazione/recupero effettivamente richiamati dalla suite. |
| Build e distribuzione | Manifest e lockfile Cargo/npm, Vite, Dockerfile, Compose, `.dockerignore`, packaging, notices, workflow Checks/Docker/load e Dependabot: input bloccati, confini PR/pubblicazione, requisiti CI e sorgenti distribuiti. |
| Documentazione | README, guide tecniche in `docs/`, istruzioni agenti, ADR, glossario e mappa GitHub: confrontati con i percorsi eseguibili; corretti i fatti CI obsoleti e la formattazione della mappa. |

## Riscontri confermati e correzioni

### 1. Upload interrotti: file temporanei mai recuperati

- Posizione: `Storage::receive_locked` in `src/jobs.rs`; `cleanup` in
  `src/retention.rs`.
- Categoria e impatto: spazio; una future di ricezione annullata salta la
  rimozione finale di `spool/incoming`. Riavvii/upload interrotti possono
  accumulare file senza inventario e senza un percorso di pulizia.
- Evidenza: il test `cancelled_upload_does_not_leave_incoming_bytes_after_cleanup`
  avvia una ricezione incompleta, verifica che la pulizia attenda il ricevitore,
  annulla la task e invoca la pulizia. Prima della correzione rimaneva un file
  (`left: 1`, `right: 0`).
- Correzione: eliminazione dei soli file ordinari in `spool/incoming` sotto il
  gate esclusivo già condiviso con tutti i ricevitori. Nessuna rimozione ricorsiva.

### 2. Pubblicazione interrotta: file immutabili senza inventario

- Posizione: pubblicazione tramite hard link in `src/jobs.rs`; pulizia in
  `src/retention.rs`.
- Categoria e impatto: spazio e recupero dopo errore; se l'inserimento in
  `source_files` fallisce dopo la pubblicazione del file, la vecchia pulizia,
  guidata esclusivamente dalle righe del database, non lo trova mai.
- Evidenza: `failed_source_publication_does_not_leave_untracked_spool_bytes`
  inietta un errore SQLite prima dell'inserimento dell'inventario, dopo che i
  byte sono stati pubblicati. Prima della correzione falliva con
  `untracked published bytes are never reclaimed`.
- Correzione: enumerare i file realmente presenti nello spool e verificare
  l'assenza di una riga ancora conservata prima di rimuoverli, sempre sotto il
  gate esclusivo. Il nome deve essere un hash; directory e link non vengono
  rimossi. Le dipendenze delle vecchie copie tecniche restano protette.

### 3. Pulizia periodica: lavoro ripetuto sullo storico concluso

- Posizione: query dei candidati e fase di unlink in `src/retention.rs`.
- Categoria e impatto: CPU, RAM e I/O. La query precedente confrontava ogni
  sorgente con i lavori e con più sottoquery correlate; includeva anche sorgenti
  già rilasciate. A ogni ciclo si ricostruiva inoltre un vettore di tutti gli hash
  rilasciati, tentando di cancellare anche file già assenti.
- Evidenza: microbenchmark in memoria con tutte le migrazioni, 1.500 sorgenti
  distinte e un lavoro completato per sorgente. Stessa macchina, Python 3.14.7 /
  SQLite 3.50.4, una esecuzione per caso:

  | Stato dei file | Query precedente | Query corretta | Candidati prima/dopo |
  | --- | ---: | ---: | ---: |
  | Conservati | 0,5958 s | 0,0030 s | 1.500 / 1.500 |
  | Già rilasciati | 0,5970 s | 0,0002 s | 1.500 / 0 |

  È una misura isolata della query, non il tempo end-to-end dell'applicazione
  né una misura della sua SQLite collegata. Fixture: `id=content_hash` da
  `format(n, '064x')`, `size_bytes=1`, un job `completed` con `file_id=id`;
  secondo caso ottenuto impostando tutti i `retained=0`. La query precedente è
  recuperabile dal commit base; quella corrente vive in `cleanup`.
- Correzione: unione delle quattro provenienze dei riferimenti, aggregazione
  per file, verifica che **tutti** i lavori siano conclusi con esito compatibile,
  esclusione dei già rilasciati. Unlink guidato dai file ancora su disco, senza
  il vettore dello storico. Nessun nuovo indice o migrazione.
- Garanzie verificate: riferimenti diretti, `job_inputs`, `job_members` e
  `listenbrainz_pages`; condivisione fra lavori; sorgenti non ancora collegate;
  stati pendenti/falliti/parziali/cancellati; `no_music`; riavvio dopo il commit
  del rilascio; pin delle copie legacy; reupload e idempotenza.

### 4. Grafici delle abitudini: tutte le righe in RAM prima di aggregare

- Posizione: `get` in `src/listening_patterns.rs`.
- Categoria e impatto: RAM; `fetch_all` conservava timestamp, fuso e JSON dei
  contesti di ogni ascolto selezionato fino alla fine dell'aggregazione.
- Evidenza: percorso HTTP e durata delle righe verificati nel codice; i test
  Rust/browser esistenti controllano filtri, campi mancanti/conflittuali e serie.
  Non viene attribuita una riduzione RSS numerica senza una misura del processo.
- Correzione: consumo incrementale dello stream SQLx e conteggio durante
  l'aggregazione. Restano in memoria le serie, le categorie e i fusi incontrati;
  eliminato il vettore di tutte le righe. Totali, copertura, ordinamento e limite
  delle categorie in risposta mantengono lo stesso contratto.

### 5. Packaging: hash dell'intero archivio in una singola allocazione

- Posizione: hash finale in `scripts/package-distribution.py`.
- Categoria e impatto: RAM durante la build dell'immagine; `read_bytes()`
  caricava l'intero `source.tar.gz`, inclusi vendor e strumenti npm.
- Evidenza: fixture sintetica di 64 MiB, stesso SHA-256 prima/dopo; picco delle
  allocazioni Python misurato con `tracemalloc`: 67.109.098 byte prima,
  394.321 byte dopo. Non è il picco RSS dell'intero packaging.
- Correzione: `hashlib.file_digest` su file aperto in lettura, disponibile nel
  Python 3.11 della base Debian usata dal builder. Archivio, contenuto e formato
  del checksum non cambiano; il packaging completo è verificato dal job Docker.

### 6. Dipendenza diretta obsoleta dopo l'aggiornamento Argon2

- Posizione: `Cargo.toml`, `Cargo.lock`.
- Categoria e impatto: codice/configurazione non necessari, build e dimensioni
  dei sorgenti distribuiti. `password-hash 0.5` non era usato; autenticazione e
  recupero importano i tipi esposti da Argon2, che dipende da `password-hash 0.6`.
- Evidenza: ricerca degli usi e `cargo tree --locked -i password-hash@0.5.0`
  mostravano il solo collegamento diretto da scrobbl-rs.
- Correzione: eliminata la dipendenza diretta. Il lockfile perde anche
  `rand_core 0.6.4`, rimasto senza utilizzatori. Nessun cambio di algoritmo o
  politica password; test di accesso, cambio password e recupero mantenuti.

### 7. Documentazione CI e Markdown della mappa

- Posizione: `docs/ci.md` e corpo della mappa GitHub.
- Categoria e impatto: documentazione obsoleta/illeggibile. La guida non
  descriveva il detector per sole modifiche documentali e confondeva Node 24
  della CI con Node 25 del Dockerfile. La mappa aveva titoli, elenchi e righe di
  tabella concatenati, oltre a sequenze letterali `\n`/`\r`.
- Correzione: guida riallineata al workflow attuale; Markdown della mappa
  riformattato con verifica dell'identità del contenuto non costituito da spazi
  dopo la decodifica delle sequenze di newline. Nessuna decisione di prodotto
  modificata.

### 8. Reimport ListenBrainz in concorrenza con la pulizia

- Posizione: ricezione multipart e salvataggio delle pagine pubbliche in
  `src/listenbrainz.rs`; enqueue in `src/jobs.rs`.
- Categoria e impatto: correttezza/concorrenza. La ricezione proteggeva il file
  solo fino alla registrazione nell'inventario. Prima del collegamento al nuovo
  lavoro, la pulizia poteva vedere soltanto i precedenti lavori completati e
  rilasciare il file appena ricaricato. L'upload terminava con HTTP 409; la stessa
  finestra esisteva fra ricezione di una pagina pubblica e `listenbrainz_pages`.
- Evidenza: il test HTTP
  `reupload_attaches_its_job_before_waiting_cleanup_can_release_the_source`
  importa un file, lo rilascia, blocca il writer durante il reupload e accoda
  precisamente la pulizia dietro la ricezione. Prima della correzione risponde
  409 invece di 200. Non richiede richieste di rete esterne o stress probabilistico.
- Correzione: un solo lease dello spool dalla ricezione al collegamento, sia
  per upload sia per pagine. `enqueue_locked` riusa l'implementazione esistente
  senza riacquisire il read lock dietro un writer già in attesa, evitando anche
  un possibile deadlock da lock annidati. Il test passa e il file rimane presente.

### 9. Test di carico ZIP con conteggio globale obsoleto

- Posizione: `full_size_spotify_zip` in `tests/spotify_import.rs`.
- Categoria e impatto: test obsoleto; il workflow manuale di carico falliva
  benché tutti i 18.000 ascolti fossero importati correttamente. Il conteggio
  globale dei membri includeva i due elementi della coda immagini, aggiunta
  successivamente all'import: 20 contro i 18 attesi.
- Correzione: contare i membri del lavoro d'import esaminato tramite `job_id`,
  mantenendo l'asserzione sui 18 file. La copertura della coda immagini rimane
  nei test dedicati; non è stato alterato il comportamento dell'importatore.

## Sospetti scartati e scelte mantenute

- **SQLx e rusqlite come duplicazione:** la doppia interfaccia serve per
  eseguire batch limitati sul medesimo handle esclusivo, evitando un passaggio
  di worker per ogni piccolo statement. Condividono `libsqlite3-sys`; nessuna
  prova di un vantaggio nel rimuoverne una, quindi mantenute.
- **Doppia lettura degli import Spotify:** la prima passata valida/classifica
  prima dei commit e stabilisce i conteggi. Non è una duplicazione eliminabile
  senza riconsiderare le garanzie dell'import; parser, canale e batch sono già
  limitati per righe/byte.
- **Migrazioni del prototipo, payload di compatibilità e vecchi helper:** non
  rimossi per somiglianza con codice recente. Migrazioni applicate e restore
  legacy sono contratti di compatibilità; i helper CJS sono richiamati dai test.
- **Immagini Spotify permanenti e copie di sicurezza:** la crescita è prevista
  dalla scelta del proprietario e dall'ADR. Non introdotti scadenze, budget,
  pulizie di backup o `VACUUM` periodici che cambierebbero quelle garanzie.
- **Frontend:** cache delle letture limitata a 64 voci, download immagini con
  uno slot, polling sospeso a documento nascosto e cleanup dei principali
  listener/observer verificati. Nessun nuovo difetto confermato richiede un
  cambiamento del codice visuale in questo audit.
- **Test, CI e distribuzione:** nessun ulteriore difetto confermato oltre ai
  riscontri sopra. Nomi dei sette check obbligatori riscontrati nella protezione
  di main; PR senza segreti di pubblicazione; pubblicazione separata dell'immagine
  testata e solo da push fidati. Non modificati i workflow.

## Verifiche

Le verifiche locali e l'esito della CI del commit consegnato sono riepilogati
anche nel commento di risoluzione del ticket, con i link alla PR e ai check.

- Rustfmt e Clippy con warning negati: riusciti. Suite Rust all-targets:
  **143 test passati**, 5 opt-in ignorati nel giro ordinario.
- Eseguiti separatamente i due carichi sintetici opt-in: 100.000 ascolti /
  5.000 brani (inclusi HTTP, backup, calendario e snapshot), e ZIP da
  124.396.022 byte / 18 JSON / 18.000 righe. Entrambi riusciti. Nel secondo,
  elaborazione dell'import in 34,44 s; nel primo, archivio finale di
  421.498.880 byte. Sono misure informative in build di sviluppo, con altri
  controlli in esecuzione, non un benchmark isolato del server di destinazione.
- TypeScript e build Vite riusciti; asset incorporati e contratti generati
  senza differenze. Chromium locale: **68 scenari passati**; il giro precede
  l'ultimo fix di sincronizzazione ListenBrainz, verificato dal nuovo test HTTP
  e dalla suite Rust completa. La CI browser verifica l'insieme finale.
- I due test dei file abbandonati e quello HTTP di concorrenza sono stati
  osservati fallire prima della correzione e passare dopo. Ulteriore test
  controlla tutti i tipi di riferimento alle sorgenti.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets -- --nocapture
cargo test --locked --test foundations foundation_load -- --ignored --exact --nocapture
cargo test --locked --test spotify_import full_size_spotify_zip -- --ignored --exact --nocapture
npm run typecheck
npm run build
git diff --exit-code -- static/dist
npm run test:e2e -- --browser=chromium
git diff --check
```

Ambiente locale: Windows x86_64, Rust 1.99.1, Node 26.10.0, build Rust di
sviluppo. Linux, Node 24, Firefox, WebKit e immagine linux/amd64 sono coperti
dalla CI richiesta della PR; i risultati vanno letti sul suo ultimo commit.

## Limiti e seguito

Il collaudo sul server privato, PC/Mac/iOS e con Spotify reale rimane in
[Collaudare e pubblicare la prima release](https://github.com/itsmrma/scrobbl-rs/issues/31);
la verifica con il prossimo export rimane in
[Verificare riconciliazione e timestamp con il prossimo export esteso
Spotify](https://github.com/itsmrma/scrobbl-rs/issues/18). L'audit non li sostituisce.

La cardinalità delle categorie dei grafici e i dati normalizzati crescono con
l'archivio; lo streaming non rende costante ogni componente della memoria.
Le copie API sono limitate dalle quote; backup e immagini permanenti rimangono
scelte esplicite dell'utente. Un'ulteriore riduzione richiederebbe misure su un
carico rappresentativo e, per cambiare la conservazione, una nuova decisione.
Non sono stati introdotti refactor generali o rimosse funzionalità concordate.

# Audit del Codebase: Pulizia del Codice Obsoleto e Ottimizzazioni

Data: 26 ottobre 2026.  
Base esaminata: `0a79b54` (branch `main`).  
Branch di lavoro: `refactor/codebase-audit-cleanup`.

---

## 1. Metodologia e Perimetro

L'audit ha esaminato integralmente l'albero dei 315 file tracciati del repository `itsmrma/scrobbl-rs` per individuare:
1. Codice duplicato, obsoleto o ridondante.
2. Residui e logiche legate al recupero di artwork da Spotify e all'applicazione dei relativi badge "Spotify" sulle copertine.
3. Inefficienze architetturali, allocazioni superflue, chiamate N+1 o polling non necessario.
4. Opportunità concrete di semplificazione e velocizzazione della build e della CI.

Tutti gli interventi sono stati implementati mantenendo rigorosamente la compatibilità, la sicurezza crittografica e applicativa, l'integrità dei dati utente e la stabilità delle suite di test.

### Sintesi delle aree esaminate

| Area | Percorsi | Interventi principali |
| --- | --- | --- |
| **Backend & Storage** | `src/`, `release-migrations/` | Nuova migrazione `0020` per ritiro artwork temporanei Spotify; rimozione struct e campi obsoleti da DTO e query; ottimizzazioni I/O. |
| **Frontend TypeScript & React** | `frontend/src/` | Rimozione badge Spotify e chiamate N+1 in `<Cover>`; transizione a `blob:` Object URL sicuri con revoca deterministica; deduplicazione richieste `preferences`; memoizzazione selettori; caching `Intl.DateTimeFormat`. |
| **Fogli di stile** | `frontend/src/style.css` | Eliminazione di oltre 100 righe di regole CSS morte e duplicate (`.now-panel`, `.private-label`, `.image-provider-badge`, `.rating-input`, ecc.). |
| **Build, Docker & CI** | `.github/workflows/checks.yml`, `package.json` | Rimozione step e script ridondanti (`check:web`, doppi typecheck in CI, build prima dei test); restore-keys Cargo. |
| **Script & Asset** | `scripts/`, `brag-output-2026-10-24-164300/` | Allineamento tabelle in `verify-spotify-archive.py` e generazione password conforme; rimozione asset intermedi non referenziati. |

---

## 2. Ritiro Completo di Fetch e Badge Artwork Spotify

A seguito della decisione di non utilizzare più Spotify come provider per il recupero delle immagini di album e artisti:

1. **Migrazione `release-migrations/0020_retire_temporary_spotify_artwork.sql`**:
   - Ritiro permanente di tutte le righe nella tabella `artwork` aventi `bytes IS NULL OR expires_at_ms IS NOT NULL OR (source = 'spotify' AND provenance_json IS NOT NULL)`.
   - Rimozione della chiave residua `artwork_retry_at_ms` dalla tabella `settings`.
   - Pulizia degli URL copertina non più validi (`albums.cover_url = NULL`).
   - Conservazione delle copertine personalizzate (`source = 'custom'`) e delle copertine permanenti storiche preesistenti.
2. **Rimozione del provider Spotify dal backend**:
   - Eliminazione delle richieste alla Web API di Spotify per il download e il parsing di artwork.
   - Rimozione del campo obsoleto `imports` da `SpotifyStatus` e dei DTO non più utilizzati in `src/dto.rs` e `frontend/src/generated.ts`.
3. **Frontend e Badge**:
   - Eliminazione completa della classe `.image-provider-badge` e del badge "Spotify" posizionato sulle copertine.
   - Eliminazione di `ArtworkContext` e degli ascoltatori di eventi legati ai crediti delle copertine durante il rendering di elenchi e griglie.
   - Rimozione del componente `SpotifyLink` non referenziato in `frontend/src/ui.tsx`.
   - Aggiornamento dell'etichetta in modifica musica da *"Use Spotify image"* a *"Reset custom image"*.

---

## 3. Codice Obsoleto, Duplicato e Ridondante Rimosso

### Backend Rust
- Rimossi tipi e DTO inutilizzati generati per TypeScript non più referenziati da alcun client.
- Semplificata la gestione delle attività e dei report in `src/activities.rs` e `src/library.rs`.
- Rimozione di rami condizionali non più raggiungibili legati a provider artwork dismessi.

### Frontend React & TypeScript
- **Deduplicazione chiamate `api.preferences()`**: in `main.tsx` è stato corretto il doppio caricamento delle preferenze all'avvio dell'applicazione tracciando lo stato del job `calendar` con un `useRef<string | null>(null)`.
- **Condivisione costanti**: esportata la costante `WEEKDAYS` da `preferences.ts` e riutilizzata in `activities.tsx` e `settings.tsx`, eliminando array duplicati inline.
- **Funzione di formattazione centralizzata**: introdotta `formatSources` in `ui.tsx` per mappare e visualizzare in modo coerente le etichette delle sorgenti (`Spotify collection`, `Spotify import`, `ListenBrainz`), eliminando concatenazioni e `replaceAll` sparsi in `listens.tsx` e `review.tsx`.
- **Semplificazione componenti**:
  - `Now`: rimosso il ramo morto non compatto `<article className="panel now-panel">`, mantenendo la visualizzazione compatta nella barra laterale e gestendo l'espansione tramite il componente `<Cover>` integrato.
  - `Heading`: rimosso il prop opzionale `eyebrow` ormai deprecato da tutte le viste e dalle definizioni.
  - `SkeletonPlayback`: rimosso il ramo non compatto ridondante.
  - `DataRecovery`: rimossa la sezione duplicata *Recent imports* che interrogava un endpoint non più manutenuto per tale scopo.

### Fogli di stile (`frontend/src/style.css`)
- Rimossi blocchi CSS orfani e duplicati:
  - Regole duplicate per `.api-secret` e `.api-secret .connection-actions`.
  - Regola duplicata per `fieldset { min-width: 0; }`.
  - Classi morte: `.private-label`, `.stat abbr`, `.now-panel`, `.now-content`, `.entity-filter .pill`, `.activities-panel > label`, `.rating-input`, `.rankings-panel > .filters`, `.history-filter-heading`.
  - Regola `.image-provider-badge` e relativo selettore `:has(.image-provider-badge)`.

### Build, CI e Script
- **`package.json`**: rimosso lo script duplicato `"check:web"` (identico a `"typecheck"`).
- **`.github/workflows/checks.yml`**:
  - Aggiunto `restore-keys:` per massimizzare il riuso della cache di Cargo tra commit vicini.
  - Rimosso lo step ridondante `cargo build --locked --all-targets` nel job `backend`, già eseguito implicitamente da `cargo test`.
  - Rimosso il doppio `npm run typecheck` che precedeva immediatamente `npm run build` (che include già il controllo dei tipi via `tsc`).
- **`scripts/verify-spotify-archive.py`**:
  - Aggiornato l'elenco delle tabelle in `archive_fingerprints` includendo `name_groups`, `name_group_members`, `artwork` e `artwork_misses`.
  - Implementata la funzione `generate_password()` per generare password casuali conformi a tutti i requisiti di complessità di `src/auth.rs::validate_password` (presenza di maiuscole, minuscole, numeri e caratteri speciali alternati per evitare sequenze alfabetiche o numeriche consecutive).
- **Asset HyperFrames**:
  - Eliminati i file intermedi non referenziati nella cartella `brag-output-2026-10-24-164300/` che appesantivano la cronologia Git, conservando esclusivamente `brag.mp4` e `brag.gif` richiamati nel `README.md`.

---

## 4. Inefficienze e Ottimizzazioni Concrete

1. **Eliminazione del problema N+1 su `<Cover>`**:
   - In precedenza, ogni istanza di `<Cover>` scatenava un `api.artworkCredit(url)` all'evento `onLoad`. In pagine con decine o centinaia di copertine (classifiche, storico, collage), venivano effettuate fino a 100 richieste HTTP superflue all'endpoint `/credit`.
   - Le informazioni di attribuzione vengono ora richieste unicamente dalla pagina di dettaglio brano/album/artista (`MusicActions` via `libraryDetails`) e dalla funzione di export collage.
2. **Streaming e lifecycle dei Blob URL**:
   - `<Cover>` ora ottiene un `blob:` Object URL al download iniziale e lo assegna direttamente al tag `<img>`, evitando una seconda richiesta HTTP `?cached=true&downloaded=true`.
   - Il ciclo di vita dell'Object URL è gestito con cleanup rigoroso tramite `URL.revokeObjectURL(ready.src)` sia allo smontaggio del componente che in caso di cambio copertina o annullamento.
3. **Caching `Intl.DateTimeFormat`**:
   - Le funzioni di formattazione data e periodi (`date` in `ui.tsx` e `periodLabel` in `consultation.tsx`) memorizzano in cache (`Map<string, Intl.DateTimeFormat>`) le istanze dei formattatori internazionali basate su timezone e opzioni, evitando l'allocazione ripetuta di oggetti `Intl` su ogni riga di ascolto.
4. **Memoizzazione in `Select` e `Collage`**:
   - In `select.tsx`, i valori derivati dai nodi figli JSX e le opzioni filtrate sono stati racchiusi in `useMemo`, eliminando ricalcoli e riallocazioni a ogni render del parent.
   - In `collage.tsx`, il calcolo del totale dei play/tempo ascoltato è stato spostato in un `useMemo` all'esterno del loop di rendering della legenda, e la troncatura delle etichette su Canvas è stata resa sicura per i code point Unicode.
5. **Controllo del Polling in Background**:
   - L'intervallo dell'orologio in `Now` viene sospeso quando la sidebar è collassata o in background (`paused`).
   - La visualizzazione `Recovery` sospende il polling degli asset quando la tab delle impostazioni non è attiva (`active = false`).

---

## 5. Falsi Positivi Scartati (Do Not Break)

Durante l'audit sono state attentamente valutate alcune potenziali aree di semplificazione, decidendo motivatamente di **non** procedere per preservare sicurezza e contratti:

1. **Client HTTP per-hop in `library::public_response`**:
   - *Ipotesi iniziale*: Riutilizzare un `reqwest::Client` globale condiviso invece di istanziarne uno per richiesta.
   - *Motivo del mantenimento*: La funzione risolve e valida preventivamente gli indirizzi IP pubblici, istanziando il client con `.resolve_to_addrs(host, &addresses)` per ogni singolo hop. Questo vincolo previene attacchi di tipo DNS Rebinding e SSRF verso reti interne (TOCTOU). Il client per-hop è stato rigorosamente preservato.
2. **API pubbliche di base in `consultation.rs`**:
   - *Ipotesi iniziale*: Rimuovere `Storage::changes`, `Storage::prune_changes` e le struct `Change` e `Position` perché non richiamate direttamente dalle rotte HTTP correnti.
   - *Motivo del mantenimento*: Si tratta delle interfacce del contratto architetturale di fondazione dell'archivio, verificate e garantite da test di integrazione essenziali (`tests/archive.rs`, `tests/foundations.rs`).
3. **Migrazioni SQL storiche `0001..0019`**:
   - Le migrazioni già committate non sono state toccate per preservare la validità degli hash SHA-384 verificati da SQLx all'avvio. Le modifiche allo schema e la pulizia delle righe sono state incapsulate nella migrazione incrementale `0020`.
4. **Rimozione selettiva in `recovery.rs`**:
   - Il comando `DELETE FROM artwork WHERE source='spotify'` nei backup portabili è stato mantenuto: garantisce che le vecchie immagini Spotify permanenti non vengano incluse negli export portabili destinati ad altre istanze.

---

## 6. Verifiche Eseguite ed Esiti

Tutte le modifiche sono state verificate localmente con successo:

```bash
# 1. Controllo formattazione Rust
cargo fmt --all -- --check
# Esito: OK (0 differenze)

# 2. Controllo linter Clippy
cargo clippy --locked --all-targets -- -D warnings
# Esito: OK (0 errori, 0 warning)

# 3. Controllo tipi TypeScript
npm run typecheck
# Esito: OK (0 errori)

# 4. Compilazione asset web di produzione
npm run build
# Esito: OK (static/dist generato in 223ms)

# 5. Suite completa dei test unitari e di integrazione Cargo
cargo test --locked --all-targets
# Esito: OK (tutti i test passati)
```

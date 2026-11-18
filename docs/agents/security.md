# Guida alla Sicurezza e Architettura Anti-Slop per Agenti

Questo documento definisce le garanzie di sicurezza di **scrobbl.rs** e i vincoli architetturali deterministici che ogni agente e sviluppatore **deve** rispettare prima di modificare o aggiungere funzionalità.

Nessuna modifica può aggirare questi confini (nessun `#[allow(...)]` ingiustificato, nessun accesso diretto ai file DB al di fuori dei moduli preposti, nessuna chiamata HTTP o SQL non filtrata).

---

## 1. Superficie di attacco e invarianti di sicurezza

### 1.1 Ask: solo modelli SQL locali e Sandbox SQLite
- **Solo modelli locali `.gguf`**: Ask non supporta più server AI remoti (BYO AI), server MCP né domande guidate DSL. Opera esclusivamente traducendo domande in linguaggio naturale in query SQL `SELECT`/`WITH` di sola lettura tramite modelli locali (Qwen3.5 dal catalogo curato o un file `.gguf` locale indicato tramite `ai_custom_model_path` / `ai_model_dir`).
- **Separazione fisica dei database**: I modelli AI non interrogano **mai** `data/archive.db` né `data/private/auth.db`. Ogni query generata dal modello o modificata dall'utente passa **esclusivamente** da `src/ask/sandbox.rs::execute` su `ask_projection.db`, aperto in sola lettura (`SQLITE_OPEN_READ_ONLY`, `query_only = ON`, `trusted_schema = OFF`, `DEFENSIVE`, estensioni disabilitate).
- **Difesa in profondità nella sandbox (`src/ask/sandbox.rs`)**:
  - Controllo statico pre-prepare UTF-8 safe (`char_indices()`, mai byte slicing diretto `&sql[i..j]`), limite 8 KB, rifiuto NUL byte, rifiuto parametri (`?`, `:x`, `@x`, `$x`), singolo statement `SELECT`/`WITH`, tracking delle CTE dichiarate (`WithFrame` con `AS (` esplicito).
  - Authorizer SQLite a whitelist stretta: consenti solo tabelle/colonne presenti in `projection::TABLES`, CTE dichiarate e funzioni pure whitelisted. Rifiuta esplicitamente tabelle virtuali eponime/di sistema (`sqlite_*`, `pragma_*`, `_projection_meta`, `dbstat`, ecc.).
  - Budget rigidi: timeout temporale configurabile (`SCROBBL_ASK_SQL_TIMEOUT_MS`, default 3000 ms), budget istruzioni VDBE/progress handler, tetto a 1.000 righe e 1 MB di output, rifiuto di `BLOB`.
  - Sanitizzazione errori: nei round di riparazione e nei log non vanno **mai** esportati valori letti dal DB, solo classe di errore e identificatori SQL presenti nella query stessa.

### 1.2 Modelli GGUF locali (`src/ai/models.rs`, `src/ai/mod.rs`)
- I modelli del catalogo Qwen3.5 vengono scaricati solo da revisioni Hugging Face pinnate a compile-time con verifica esatta di dimensione e SHA-256.
- I file modello importati dall'utente (via `custom_model_path` / `ai_custom_model_path` o sottocartella dedicata dentro `ai_model_dir`) sono trattati come **input non fidato**:
  - `validate_gguf_file` verifica che il percorso esista, **non** sia un symlink (`symlink_metadata`), sia un file regolare con estensione `.gguf`, dimensione minima di 64 byte e intestazione binaria esatta `b"GGUF"`.
  - `models::scan` ignora directory arbitrarie dell'host che non contengano né `metadata.json` né un singolo `.gguf` valido.
  - `models::remove` non usa **mai** `remove_dir_all` cieco su directory generiche: rifiuta symlink e sottodirectory annidate e rimuove solo file modello attesi (`metadata.json`, `.gguf`, `.gguf.part`).

### 1.3 HTTP, Autenticazione, CSRF e Ordinamento dei Lock (`src/web.rs`, `src/auth.rs`, `src/account.rs`, `src/boot.rs`)
- **Ordine dei controlli nel middleware `security` (`src/web.rs`)**:
  1. Blocca `Authorization` sulle route interne (ammesso solo su `/api/read/*`).
  2. Verifica rigorosa di header `Host` (e `Origin` sulle mutazioni) prima di accedere a risorse stateful.
  3. Verifica sessione + CSRF (`auth::require_write`) **prima** di acquisire il lock globale delle richieste (`storage.requests`) o leggere marker di stato costosi, per impedire DoS pre-autenticazione sui lettori o sul restore.
- **Sessioni e password**:
  - Tentativi di login/setup e cambi password usano secchielli di rate-limiting indipendenti (`auth_attempts` e `password_attempts` in `AppState`).
  - Lookup sessione su **tutti** gli header `Cookie` della richiesta (`session_cookies`), confronto CSRF constant-time, cap a 64 sessioni attive per utente.
- **Intestazioni di sicurezza**:
  - Ogni risposta HTTP — incluse quelle di errore, risposte del Read API esterno e pagine di avvio pre-boot in `src/boot.rs` — applica `Content-Security-Policy` (con `object-src 'none'`, `frame-ancestors 'none'`, `base-uri 'none'`), `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `X-Frame-Options: DENY`, `Cross-Origin-Resource-Policy: same-origin`, `Permissions-Policy` restrittiva e `Strict-Transport-Security` quando in HTTPS (`secure_cookies`).
  - Nessun inline handler HTML (`onclick="..."`) né messaggi di errore raw con dettagli interni di sistema all'avvio (`src/main.rs`).

### 1.4 Rete in uscita & Anti-SSRF (`src/artwork_http.rs`, `src/artwork_sources.rs`)
- Nessuna chiamata HTTP esterna usa la risoluzione DNS di sistema non filtrata o redirect illimitati: `PublicResolver` valida gli indirizzi IP pubblici, limita i redirect, impone timeout espliciti e applica rate limit **prima** di risolvere il DNS su host remoti.
- Testi/crediti raccolti da fonti terze vengono normalizzati lato server (`strip_html_tags` + decodifica entità in `src/artwork_sources.rs`) e lato client (`plainCredit` in `frontend/src/image-credit.tsx`).

### 1.5 Backup e Restore (`src/recovery.rs`)
- La proiezione `ask_projection.db` e i file `.gguf` non vengono inclusi nei backup portatili.
- Durante la preparazione e validazione del restore (`sanitized_manifest`, `prepare_restore_job`), tutte le vecchie chiavi dismesse (`ai_remote_*`, `ai_provider`, `ai_answer_mode_*`) e le configurazioni dipendenti dai path host (`ai_enabled`, `ai_model_dir`, `ai_custom_model_path`) vengono azzerate (`LOCAL_ONLY_AI_SETTINGS_SQL`) per prevenire riesecuzioni indesiderate o path traversal al riavvio su un nuovo ambiente.

---

## 2. Punti Critici Anti-Slop e Blocchi Deterministici

Questa sezione documenta le 5 aree della codebase in cui un'ambiguità architetturale indurrebbe facilmente un agente AI a introdurre bug, workaround o codice fragile ("slop"), e mostra la regola deterministica che lo vieta in locale e in CI.

### 2.1 Uso di `.unwrap()` in produzione e slicing diretto dei byte su UTF-8

- **La Nuance / Il Rischio:**
  In diverse aree di test del repository (`src/activities.rs`, `src/artwork.rs`, `tests/*.rs`) gli agenti vedono un uso intenso di `.unwrap()`. Questo porta facilmente l'agente a copiare `.unwrap()` o slicing diretto `&s[a..b]` anche nel codice di produzione (es. parsing HTTP, mutex lock in `src/ask/sandbox.rs`, parsing header UTF-8 in `src/auth.rs`, analisi lessicale di query in `src/ask/sandbox.rs`). Su nomi di brani o artisti con caratteri multi-byte UTF-8 (es. `Björk`, giapponese, emoji) o input HTTP anomali, `.unwrap()` o un indice byte non allineato provoca un **panic istantaneo del thread/server (DoS)**.

- **La Soluzione Architetturale (API / Astrazione):**
  - Usare sempre propagazione strutturata con `?` / `AppError`, `.ok_or_else(...)` o `.unwrap_or_default()`.
  - Per i `std::sync::Mutex` in memoria, usare `.unwrap_or_else(|e| e.into_inner())` o gestire il `LockResult`.
  - Per scansioni lessicali o troncamenti su testo (`src/ask/sandbox.rs`, parser SQL/nomi), operare esclusivamente tramite `char_indices()`, `.chars()`, `.starts_with(...)` o `.floor_char_boundary(...)`, mai con indici byte calcolati manualmente.

- **Blocco Deterministico (Regola Linter):**
  In `Cargo.toml` e `clippy.toml`:
  ```toml
  # Cargo.toml
  [lints.clippy]
  unwrap_used = "deny"
  fallible_impl_from = "deny"

  # clippy.toml
  allow-unwrap-in-tests = true
  ```
  Il check CI esegue `cargo clippy --locked --all-targets -- -D warnings`. Qualsiasi `.unwrap()` in codice di produzione fallisce deterministicamente la build segnalando `clippy::unwrap_used`.

---

### 2.2 Aperture SQLite ad-hoc (`Connection::open*`) fuori dai boundary di Storage e Ask Projection

- **La Nuance / Il Rischio:**
  La dipendenza `rusqlite` è esposta nel crate perché la sandbox SQL e il builder della proiezione ne hanno bisogno a basso livello (`src/ask/projection.rs`, `src/ask/sandbox.rs`). Un agente che deve leggere un'informazione veloce potrebbe essere tentato di chiamare `rusqlite::Connection::open(path)` o `Connection::open_with_flags(...)` direttamente in un handler web o in un helper (`src/ask/history_link.rs`, nuovi moduli), creando connessioni prive dei flag di difesa (`SQLITE_OPEN_READ_ONLY`, `query_only = ON`, `trusted_schema = OFF`, `DEFENSIVE`, disabilitazione `load_extension`) o accedendo al DB sbagliato senza passare per il pool principale.

- **La Soluzione Architetturale (API / Astrazione):**
  - Per il database principale dell'archivio (`archive.db`) e dell'autenticazione (`auth.db`), usare **sempre** il layer asincrono e i pool gestiti da `src/storage.rs` / `AppState`.
  - Per letture applicative read-only sulla proiezione Ask (query interne con parametri bindati), usare l'unica astrazione `crate::ask::projection::open_read_only(projection_path)`.
  - Per qualunque query SQL scritta dal modello o dall'utente, passare **obbligatoriamente** da `crate::ask::sandbox::execute(...)`.

- **Blocco Deterministico (Regola Linter):**
  In `Cargo.toml` (`disallowed_methods = "deny"`) e `clippy.toml`:
  ```toml
  disallowed-methods = [
    { path = "rusqlite::Connection::open", reason = "Use crate::storage::open(...) or crate::ask::projection::open_read_only(...) (or annotate the projection/sandbox seam directly); never open ad-hoc SQLite connections without security PRAGMAs." },
    { path = "rusqlite::Connection::open_with_flags", reason = "Use crate::storage::open(...) or crate::ask::projection::open_read_only(...) (or annotate the projection/sandbox seam directly); never open ad-hoc SQLite connections without security PRAGMAs." },
  ]
  ```
  Solo gli ingressi infrastrutturali verificati (`projection.rs`, `sandbox.rs`) possono annotare esplicitamente `#[expect(clippy::disallowed_methods)]`.

---

### 2.3 Client HTTP in uscita non configurati (`reqwest::Client::new` / `reqwest::get`)

- **La Nuance / Il Rischio:**
  Aggiungere una chiamata HTTP esterna con `reqwest::get(url)` o `reqwest::Client::new()` (come accadeva nell'healthcheck CLI o in nuovi fetcher) è un pattern rapidissimo per un agente, ma disabilita silenziosamente la protezione da SSRF (`PublicResolver`), lascia attivi eventuali proxy di sistema imprevisti, segue redirect verso `file://` o reti private (`127.0.0.1`, `169.254.169.254`, RFC 1918) e non imposta timeout su connessione e lettura bloccando i worker Tokio.

- **La Soluzione Architetturale (API / Astrazione):**
  - Per fetch verso servizi/URL terzi, passare attraverso il client condiviso e protetto da DNS pinning in `src/artwork_http.rs` (o estenderne il builder con `PublicResolver`).
  - Per client locali strettamente necessari (es. loopback `--healthcheck` in `src/main.rs`), usare sempre `reqwest::Client::builder()` impostando esplicitamente `.no_proxy()`, `.timeout(...)`, `.connect_timeout(...)` e `.redirect(reqwest::redirect::Policy::none())` (o policy custom validata).

- **Blocco Deterministico (Regola Linter):**
  In `clippy.toml`:
  ```toml
  disallowed-methods = [
    { path = "reqwest::Client::new", reason = "Use reqwest::Client::builder() with explicit .timeout(...), .connect_timeout(...), .no_proxy(), and redirect / SSRF validation (see src/artwork_http.rs)." },
    { path = "reqwest::get", reason = "Never use unconfigured reqwest::get; build an explicit reqwest::Client with timeouts, redirect limits, and .no_proxy()." },
  ]
  ```

---

### 2.4 Chiamate `fetch()` sparse e sink DOM non sicuri nei componenti React (`frontend/src/*`)

- **La Nuance / Il Rischio:**
  Nei componenti React/TS (`frontend/src/lifecycle-screen.tsx`, `frontend/src/main.tsx`, `frontend/src/image-credit.tsx`), in passato erano presenti chiamate `fetch("/api/boot-status")` dirette e manipolazioni DOM tramite `template.innerHTML = value`. Quando un agente crea una nuova schermata o un widget, tende a replicare `fetch(...)` dentro `useEffect` (saltando l'invio automatico dell'header `X-CSRF-Token`, la gestione uniforme di sessione scaduta `SessionExpired` e l'abort su smontaggio) oppure a usare `.innerHTML` / `dangerouslySetInnerHTML` per "riulire" stringhe provenienti da terze parti, esponendo la UI a XSS e DOM clobbering.

- **La Soluzione Architetturale (API / Astrazione):**
  - Ogni richiesta HTTP verso il backend deve passare attraverso una funzione tipizzata in [`frontend/src/api.ts`](../../frontend/src/api.ts) (`api.bootStatus()`, `request`, `post`).
  - Il testo proveniente da metadati esterni va sanificato con helper puri su stringa come `plainCredit()` in [`frontend/src/image-credit.tsx`](../../frontend/src/image-credit.tsx) e gli `<a href>` esterni vanno sempre filtrati tramite `safeExternalUrl()` in [`frontend/src/ui.tsx`](../../frontend/src/ui.tsx) (ammessi solo `http:` e `https:`).
  - Upload e link interni devono validare rispettivamente MIME/dimensione (`frontend/src/music-edit.tsx`) e path relativo sicuro senza protocolli/`//` (`frontend/src/navigation.tsx`).

- **Blocco Deterministico (Regola Linter):**
  In [`scripts/lint-frontend.cjs`](../../scripts/lint-frontend.cjs) (invocato da `npm run lint:web`, `npm run build` e dalla job Browser in `.github/workflows/checks.yml`):
  - `[no-raw-fetch]` blocca ogni `\bfetch\s*\(` al di fuori di `frontend/src/api.ts`.
  - `[no-unsafe-dom-sink]` blocca `.innerHTML`, `.outerHTML`, `insertAdjacentHTML(`, `dangerouslySetInnerHTML` ed `eval(`.

---

### 2.5 Indicizzazione diretta `MAP[key]` su lookup costanti in TypeScript (`Object.prototype` pollution / leak)

- **La Nuance / Il Rischio:**
  In TypeScript è naturale definire mappe costanti `const DEVICES: Record<string, string> = { ... }` (`frontend/src/ui.tsx`, `review.tsx`, `startup-screen.tsx`, `image-credit.tsx`, `ai-settings.tsx`) e leggerle con `DEVICES[key] || fallback`. Ma se `key` arriva da un provider esterno, dal backend o da un campo DB (es. `provider = "constructor"` o `error_code = "toString"`), `DEVICES[key]` non è `undefined`: restituisce la funzione nativa di `Object.prototype`, che in JSX fa fallire il rendering o aggira i branch di fallback. Un problema analogo si verifica accedendo per indice a tuple/array come `WEEKDAYS[n]` senza bounds check.

- **La Soluzione Architetturale (API / Astrazione):**
  - Usare sempre `lookupRecord(MAP, key)` esportato da [`frontend/src/ui.tsx`](../../frontend/src/ui.tsx), che verifica `Object.hasOwn(map, key)` prima di leggere il valore.
  - Per array indicizzati da numeri (es. `WEEKDAYS` in [`frontend/src/activities.tsx`](../../frontend/src/activities.tsx)), usare `.at(index) ?? fallback`.

- **Blocco Deterministico (Regola Linter):**
  In [`scripts/lint-frontend.cjs`](../../scripts/lint-frontend.cjs), la regola `[no-unchecked-uppercase-map-index]` rifiuta qualsiasi accesso diretto `UPPER_CASE_NAME[...]` su tutti i file `frontend/src/**/*.{ts,tsx}`, istruendo l'agente a sostituirlo con `lookupRecord(NAME, key)` o `.at(index)`.

---

## 3. Checklist prima di ogni PR

1. **Solo Ask locale SQL:** nessuna reintroduzione di server esterni AI, API key remote, MCP o fallback DSL. Se si aggiunge un modello al catalogo o si tocca il caricamento `.gguf`, verificare che `validate_gguf_file` e `schema::sql_schema()` restino invariati come confine unico.
2. **Database & Rete:** zero `rusqlite::Connection::open*` fuori da Storage/Projection/Sandbox, zero `reqwest::Client::new`/`get`, zero accessi filesystem non validati (no symlink, path relativi confinati).
3. **Frontend & Validazioni:** zero `fetch()` fuori da `frontend/src/api.ts`, zero `.innerHTML`/`dangerouslySetInnerHTML`/`eval()`, tutti gli URL esterni passati da `safeExternalUrl()`, tutte le mappe `Record` lette con `lookupRecord()`.
4. **Verifiche locali obbligatorie prima del push:**
   ```sh
   npm run build
   cargo fmt --check
   cargo clippy --locked --all-targets -- -D warnings
   cargo test --locked --all-targets
   ```

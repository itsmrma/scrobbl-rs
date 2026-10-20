# scrobbl.rs — progetto e stato dei lavori

Ultimo aggiornamento: 20 ottobre 2026.

- Nome del prodotto: **scrobbl.rs**.
- Repository: <https://github.com/itsmrma/scrobbl-rs>.
- Nome per repository e futura immagine Docker: `scrobbl-rs`.
- Stato: prima implementazione locale di backend Rust, SQLite, autenticazione, integrazione Spotify,
  import dello storico, dashboard e cronologia. Prova reale su account Spotify ancora da completare.
  Pagina delle chiavi API, integrazione con spotify_analysis e container non ancora implementati.

Questo documento permette di riprendere il lavoro senza ricostruire la conversazione. Le
funzionalità indicate come confermate sono state scelte dall'utente; le proposte tecniche e le
questioni aperte sono riportate separatamente.

## 1. Obiettivo

Realizzare uno scrobbler e un archivio personale di ascolti, self-hosted, scritto in Rust e
fortemente ispirato a [Maloja](https://github.com/krateng/maloja).

La prima versione è dedicata a **Spotify**, con collegamento diretto all'account, importazione dello
storico e raccolta automatica dei nuovi ascolti, sul modello di ListenBrainz e stats.fm. Altri
servizi potranno essere aggiunti in futuro.

L'applicazione è destinata a un solo utente per installazione. Altre persone potranno scaricarla e
avviare la propria istanza con dati, credenziali Spotify e chiavi API indipendenti. Non è previsto
un servizio centrale con account condivisi tra istanze.

## 2. Distribuzione e accesso confermati

- Codice sorgente pubblicato su GitHub.
- Immagine Docker costruita e pubblicata automaticamente tramite GitHub Actions.
- Avvio e aggiornamento sul server senza dover compilare Rust sul server stesso.
- Dati persistenti tra riavvii e aggiornamenti del container.
- Accesso iniziale **solo dalla rete locale o tramite VPN**.
- Interfaccia protetta da password anche in rete locale.
- Nessun obbligo di aprire porte del router o pubblicare il servizio su Internet.
- Eventuale esposizione pubblica tramite reverse proxy rimandata.
- Interfaccia del prodotto in **inglese**.

## 3. Raccolta degli ascolti e importazioni

### Confermato

- Collegamento diretto all'account Spotify; l'utente ha Spotify Premium.
- Raccolta automatica dei nuovi ascolti eseguita dal server anche a pagina web chiusa.
- Importazione dello storico esteso scaricato da Spotify.
- Integrazione dello storico importato con i nuovi ascolti, evitando duplicati.
- Indicazione del brano **in riproduzione**, se supportata dall'integrazione.
- Solo musica nella prima versione.
- Importazione di storici anche da **Maloja, Last.fm e ListenBrainz**.
- Distinzione tra durata effettivamente disponibile e durata stimata.

L'importazione dello storico non equivale a un collegamento continuo con Last.fm o ListenBrainz: la
raccolta diretta iniziale riguarda Spotify.

### Da verificare durante lo sviluppo

- Procedura OAuth Spotify utilizzabile su un'installazione privata in LAN/VPN.
- Requisiti correnti delle app Spotify in development mode e configurazione per ciascuna
  installazione indipendente.
- Endpoint disponibili, quote, frequenza di sincronizzazione e gestione degli errori.
- Copertura degli ascolti recenti e possibilità di recupero dopo un'interruzione.
- Campi disponibili per durata effettiva, durata del brano e stato di riproduzione.
- Riconciliazione tra fonti con timestamp o metadati diversi.

Non promettere uno storico completo ottenuto dal solo collegamento API: lo storico esteso Spotify
serve a ricostruire il passato. Non presentare la durata del brano come tempo effettivamente
ascoltato quando quel dato non è disponibile.

## 4. Statistiche e consultazione confermate

| Area                    | Comportamento desiderato                                                    |
| ----------------------- | --------------------------------------------------------------------------- |
| Dashboard               | In riproduzione, ultimi ascolti, totale ascolti e classifiche principali    |
| Classifiche             | Artisti, album e brani                                                      |
| Ordinamento             | Selettore tra numero di ascolti e tempo ascoltato                           |
| Periodi                 | Oggi, settimana, mese, anno, tutto lo storico e intervallo personalizzato   |
| Finestre mobili         | Ultimi 7, 30 e 365 giorni                                                   |
| Cronologia              | Elenco ricercabile con data, ora, copertina, brano, artisti, album e filtri |
| Pagine dedicate         | Artista, album e brano con statistiche e collegamenti                       |
| Attività nel tempo      | Grafici per giorno, settimana, mese e anno, anche per singole entità        |
| Posizioni nel tempo     | Evoluzione della posizione in classifica                                    |
| Vincitori dei periodi   | Artista, album e brano numero uno per settimana, mese e anno                |
| Medaglie annuali        | Oro, argento e bronzo per i primi tre delle classifiche annuali             |
| Settimane al numero uno | Conteggio nelle pagine dedicate                                             |

Per un brano con più artisti, attribuire **un ascolto intero a ciascun artista** nelle classifiche
degli artisti. Il brano e l'ascolto complessivo si contano una sola volta. I totali degli artisti
possono quindi superare il totale degli ascolti effettivi.

Le pagine artista devono integrare bene gli ascolti per album e per singolo brano, includendo una
vetrina degli album con copertine.

## 5. Gestione e pulizia della libreria confermate

- Correzione locale manuale dei nomi degli artisti e dei titoli di album e brani.
- Unione di artisti, album e brani duplicati, aggregando gli ascolti.
- Suggerimenti automatici per raggruppare versioni dello stesso brano presenti in album originali,
  deluxe e raccolte; applicazione solo dopo conferma dell'utente.
- Conservazione della versione originariamente ascoltata nei raggruppamenti.
- Regole automatiche per uniformare metadati e interpretare artisti e featuring, utili soprattutto
  nei dati importati.
- Eliminazione di singoli ascolti, selezioni multiple e intervalli di date.
- Pagina per individuare possibili doppioni, album mancanti e dati incompleti.

Da definire: regole precise di normalizzazione, gestione dei conflitti tra album, rielaborazione
dello storico dopo una modifica e recuperabilità delle operazioni. I metadati strutturati ricevuti
da Spotify non devono essere reinterpretati alla cieca come semplici stringhe provenienti da un
import.

## 6. Immagini e preferenze confermate

- Recupero automatico di copertine e immagini da Spotify, con eventuali fornitori alternativi quando
  mancano.
- Immagini personalizzate per artisti, album e brani da **file o URL incollato**.
- Classifiche visualizzabili come elenco compatto o griglia con copertine.
- Tema chiaro, scuro e automatico secondo il dispositivo.
- Nome dell'istanza, periodo iniziale della dashboard, formato dell'ora, fuso orario e giorno di
  inizio settimana configurabili.
- Nessuna funzione per aggiungere CSS personalizzato.

Da definire: fornitori alternativi, caching, aggiornamento delle immagini, segnaposto e trattamento
degli URL caricati dall'utente.

## 7. Collegamento con spotify_analysis

Il progetto esistente è `spotify_analysis`, nella cartella:

```text
C:\Users\mbena\Documents\spotify_analysis
```

È un'applicazione statica ospitata su GitHub Pages. Elabora i dati nel browser e dispone già di
importazioni da ListenBrainz, Last.fm e Maloja. Non ha un backend che riceve e conserva
automaticamente ascolti.

Il collegamento desiderato è:

```text
Spotify -> scrobbl.rs sul server di casa
           ^
           | API autenticata, raggiungibile tramite LAN/VPN
           |
browser dell'utente che esegue spotify_analysis caricato da GitHub Pages
```

È il **browser dell'utente**, non GitHub, a contattare il server di casa. Il server deve essere
raggiungibile dal dispositivo che apre il sito, senza essere per questo pubblicamente accessibile su
Internet.

Confermata un'API per permettere a `spotify_analysis` di leggere gli ascolti. Prevedere
un'integrazione dedicata senza obbligare a copiare il protocollo Maloja.

Riferimenti nel progetto attuale:

- `assets/js/app.js`: `downloadMaloja`, filtri temporali, paginazione e gestione errori.
- `assets/js/analytics.js`: `convertMaloja` e conversione dei dati importati.
- `tests/e2e/scrobblers.spec.cjs`: comportamento delle importazioni e degli errori.
- `README.md` e `PRODUCT.md`: dati in memoria per la sessione, gestione locale, convenzioni
  temporali e vincoli del prodotto esistente.

Non modificare implicitamente le regole di privacy o persistenza di `spotify_analysis` per
realizzare il nuovo scrobbler.

### Questioni ancora aperte

1. Aggiornamento di `spotify_analysis`: pulsante manuale, aggiornamento automatico all'apertura e
   mentre la pagina rimane aperta, oppure entrambe le possibilità.
2. Eventuale invio automatico verso altri programmi: non ancora definito; il sito statico attuale
   non offre un endpoint per riceverlo a pagina chiusa.
3. Modalità concreta di accesso HTTPS al server privato e gestione dei permessi di accesso alla rete
   locale nei browser.

## 8. Pagina delle chiavi API

È confermata una pagina autenticata per **creare e revocare API key**, inizialmente da utilizzare
con `spotify_analysis`, predisposta per altre integrazioni future.

La collocazione proposta è **Integrations -> API keys**.

### Comportamento proposto

- Creazione di una chiave con nome e indicazione dell'integrazione.
- Primo utilizzo dedicato a `spotify_analysis`; possibilità di etichettare una chiave per un'altra
  applicazione senza fingere integrazioni già implementate.
- Segreto visibile una sola volta, con pulsante per copiarlo.
- Elenco con nome, permessi, creazione, ultimo utilizzo e stato di revoca.
- Eventuale scadenza configurabile: dettaglio ancora da confermare.
- Revoca individuale e creazione di una nuova chiave in caso di smarrimento.
- URL base delle API e istruzioni per collegare `spotify_analysis`.
- Permessi iniziali limitati alla **lettura dello storico**; estendibili in futuro.
- Nessun permesso di cancellare ascolti, cambiare impostazioni, accedere ai backup o gestire il
  collegamento Spotify con una chiave di integrazione in sola lettura.

### Indicazioni tecniche proposte

- Segreti casuali ad alta entropia e conservazione di un digest sicuro sul server.
- Autorizzazione su ogni endpoint e revoca effettiva delle chiavi.
- Separazione tra sessione del proprietario e chiavi di integrazione.
- Chiavi negli header, mai nell'URL, nei log o nel codice pubblico.
- CORS configurato per origini esplicite, con gestione delle richieste preflight. CORS non
  sostituisce l'autenticazione e non distingue repository diversi sotto percorsi della stessa
  origine GitHub Pages.
- HTTPS e compatibilità con le regole dei browser anche quando si usa una VPN.
- Paginazione, filtri temporali e gestione prevedibile degli errori nelle API.
- URL da mostrare configurabile e realmente raggiungibile dal browser: non usare automaticamente un
  hostname interno del container.
- Sul client, chiave inizialmente mantenuta solo nella sessione della pagina; nessuna richiesta
  confermata di salvarla in modo persistente.

È ancora da decidere se realizzare un **esploratore interattivo delle API protetto da
autenticazione**, oltre alla documentazione nel repository.

## 9. Amministrazione e portabilità

### Confermato

- Esportazione dello storico in **JSON e CSV**.
- Backup completo e procedura di ripristino, con configurazione e immagini personalizzate.
- **Backup automatici pianificati**.
- Diagnostica con stato del collegamento Spotify, ultima sincronizzazione riuscita, errori di
  importazione e problemi da risolvere.

### Proposta di configurazione, non ancora confermata esplicitamente

- Interfaccia web per impostazioni d'uso, collegamento Spotify, aspetto e backup.
- File di configurazione e variabili d'ambiente per avvio del container, porta, percorsi persistenti
  e credenziali dell'app Spotify.
- Evitare che gli stessi parametri siano modificabili da più posti senza una precedenza chiara.

Da definire: frequenza, destinazione, conservazione e consistenza dei backup; trattamento di
credenziali e segreti nel backup e nel ripristino.

## 10. Funzionalità escluse o rimandate

| Funzione                                                                | Decisione                            |
| ----------------------------------------------------------------------- | ------------------------------------ |
| Inserimento manuale degli ascolti                                       | Escluso                              |
| Inoltro degli ascolti a Last.fm                                         | Escluso                              |
| Certificazioni oro/platino/diamante a soglie di ascolti                 | Escluse                              |
| Associazione tra gruppi, solisti e progetti paralleli nelle statistiche | Esclusa                              |
| CSS personalizzato                                                      | Escluso                              |
| Podcast e audiolibri                                                    | Fuori dalla prima versione           |
| Collegamento continuo ad altri servizi musicali                         | Rimandato                            |
| Ricezione tramite protocolli Maloja, ListenBrainz e Audioscrobbler      | Rimandata                            |
| Esposizione pubblica del server                                         | Rimandata; inizialmente solo LAN/VPN |
| Più utenti nella stessa installazione                                   | Fuori dall'impostazione attuale      |

Le medaglie annuali dei primi tre **sono incluse**: sono diverse dalle certificazioni a soglia, che
sono state escluse. Gli import storici da altri servizi **sono inclusi**: sono diversi dai
collegamenti continui e dai protocolli di ricezione rimandati.

## 11. Licenza e rapporto con Maloja

Maloja usa [GPL v3](https://github.com/krateng/maloja/blob/master/LICENSE). L'utente vuole
menzionare esplicitamente nel README che il progetto è **heavily inspired by Maloja**.

Proposta discussa: codice e interfaccia originali, identità distinta e licenza GPL v3 anche per
scrobbl.rs. **La licenza finale non è stata ancora confermata.**

L'ispirazione alle funzionalità e il riutilizzo o la traduzione di codice non sono la stessa cosa.
Se si adattano parti di Maloja, preservare gli avvisi pertinenti e rispettare gli obblighi della
GPL. La sola attribuzione nel README non li sostituisce. Controllare separatamente le licenze di
immagini, font e icone eventualmente riusati.

Se si distribuiscono binari GPL tramite immagini Docker, predisporre l'accesso ai sorgenti
corrispondenti e ai file di build per ciascuna versione pubblicata.

Bozza di riconoscimento, da adattare all'implementazione effettiva:

> Heavily inspired by [Maloja](https://github.com/krateng/maloja) by krateng. A self-hosted
> listening history and statistics application written in Rust, initially focused on Spotify.

Riferimenti discussi: [FAQ GNU](https://www.gnu.org/licenses/gpl-faq.en.html),
[GPL v3](https://www.gnu.org/licenses/gpl.en.html).

## 12. Proposte architetturali da valutare

Queste sono possibilità, non decisioni già approvate:

- SQLite come database iniziale per semplificare l'installazione a singolo utente.
- Un solo servizio applicativo con interfaccia, API e processi di sincronizzazione.
- Moduli separati per fonti musicali e integrazioni, così da aggiungerne altre.
- API nativa versionata, con adapter dedicato in `spotify_analysis`.
- GitHub Container Registry (`ghcr.io`) per le immagini Docker.
- File Docker Compose di esempio e versioni delle immagini legate alle release.

Per la prima implementazione locale sono stati scelti Rust con Axum/Tokio, SQLite con SQLx e
migrazioni SQL, frontend HTML/CSS/JavaScript servito dallo stesso processo e Reqwest per Spotify.
Non serve Node.js a runtime. Le piattaforme delle immagini Docker restano da scegliere.

## 13. Cosa rimane da fare

### Completare le decisioni

- [ ] Decidere aggiornamento manuale/automatico in `spotify_analysis`.
- [ ] Decidere se includere l'esploratore interattivo delle API.
- [ ] Confermare licenza e suddivisione della configurazione web/file/ambiente.
- [ ] Identificare sistema operativo, CPU del server e ambiente Docker.
- [ ] Definire HTTPS privato, VPN e procedura OAuth per le installazioni degli utenti.
- [x] Definire le regole iniziali per Spotify: soglia import 30 secondi, API conteggiate con durata
      stimata o sconosciuta; UTC in archivio, Europe/Rome in UI; regola di riconciliazione nella sezione 15.
- [ ] Validare i timestamp e la riconciliazione con dati Spotify reali; estendere le regole ad altre fonti.
- [ ] Definire backup e dettagli delle operazioni di correzione e raggruppamento.
- [ ] Decidere priorità e confini della prima release.

### Ordine di implementazione proposto

1. Verificare con un prototipo minimo OAuth Spotify, raccolta degli ascolti recenti e now playing:
   questa è la principale dipendenza esterna del progetto.
2. Scegliere lo stack e impostare il progetto Rust, il database e le migrazioni.
3. Realizzare autenticazione del proprietario, configurazione iniziale e persistenza.
4. Implementare import Spotify, sincronizzazione e deduplicazione.
5. Implementare API di lettura, pagina delle chiavi e accesso browser in LAN/VPN.
6. Aggiungere e verificare il collegamento in `spotify_analysis`.
7. Realizzare dashboard, cronologia, classifiche e pagine artista/album/brano.
8. Aggiungere grafici, vincitori dei periodi e medaglie annuali.
9. Implementare import da altri archivi, pulizia dei dati e immagini personalizzate.
10. Realizzare export, backup/ripristino, pianificazione e diagnostica.
11. Aggiungere verifiche mirate su autenticazione, permessi API, importazioni, deduplicazione,
    correttezza delle statistiche e ripristino dei backup.
12. Preparare Dockerfile, Compose, CI e pubblicazione delle immagini, poi verificare installazione e
    aggiornamento su un ambiente pulito.
13. Scrivere README, guida alla configurazione Spotify, guida per LAN/VPN e API, licenza e
    riconoscimenti; pubblicare la prima release.

## 14. Come riprendere la prossima sessione

Leggere questo file e controllare lo stato del repository prima di intervenire. Partire dalle
decisioni aperte e dalla verifica concreta dell'integrazione Spotify. Le scelte confermate non
devono essere richieste nuovamente, salvo nuovi vincoli tecnici da spiegare all'utente.

La prima implementazione locale è descritta nella sezione 15. Non risultano ancora pagina API keys,
modifiche all'integrazione di `spotify_analysis`, container, workflow di pubblicazione o
installazione sul server.


## 15. Prima implementazione locale — 20 ottobre 2026

L'utente ha richiesto di arrivare al prototipo Spotify, al modello dati e alla prima versione
utilizzabile, eseguendo **in locale senza Docker**. Conferme ricevute nella sessione:

- Import: conservare anche gli ascolti brevi; soglia di conteggio **30 secondi**.
- API Spotify: contare gli elementi dello storico recente; durata del brano indicata come **stimata**,
  oppure sconosciuta se mancante. Now playing non crea ascolti.
- Fuso **Europe/Rome**, settimana da lunedì, formato 24 ore.
- Accesso solo da questo computer su **127.0.0.1**, porta predefinita 3210.
- L'utente non ha ancora creato un'app Spotify: procedura nella UI e in `docs/spotify-setup.md`.
- Riconciliazione prudente: stesso URI Spotify, inizio compatibile entro **2 secondi**, un solo
  candidato e una osservazione per fonte. Inizio import ricavato da `ts - ms_played`; `played_at`
  API trattato come inizio, ipotesi da validare con dati reali. Durata reale sostituisce la stima;
  candidati multipli restano separati, segnalati nel report e nel filtro Possible duplicates della
  cronologia. Nessuna unione fuzzy per titoli.

Implementazione presente:

- Rust, Axum, Tokio, SQLx/SQLite, migrazione iniziale; frontend statico originale in inglese.
- Primo accesso con impostazione password, Argon2id, sessioni persistenti con scadenza, logout,
  controlli Origin/Host e CSRF. Password e segreti non sono inseriti nel repository.
- Configurazione Spotify dalla UI o da variabili d'ambiente; queste ultime hanno precedenza.
- OAuth con state monouso legato alla sessione, token persistenti, rinnovo e retry dopo 401,
  backoff persistente per 429 e diagnostica. Raccolta ogni 120 secondi; now playing ogni 30 secondi
  soltanto con overview aperta. Il processo deve rimanere avviato.
- Import JSON multiplo dello storico esteso (ZIP da estrarre), massimo 64 MiB per richiesta,
  transazione per file, report degli errori, record sorgente conservati e reimport idempotenti.
- Dashboard con totali e top 5 artisti/album/brani, ordinamento per ascolti o tempo, periodi calendario,
  finestre mobili e date personalizzate; cronologia con ricerca, filtri e paginazione.
- Identità import provvisorie quando mancano ID artista/album. I brani osservati dalle API acquisiscono
  crediti strutturati e copertine; riconciliazione globale di artisti/album ancora da sviluppare.
- Script di avvio `./scripts/run-local.sh`, README e guida italiana `docs/local-setup.it.md`.
- Test Rust su sicurezza, import, riconciliazione, statistiche, fuso/ora legale e Spotify simulato;
  verifica browser con archivio temporaneo separato e senza credenziali personali. Verificati
  `cargo fmt --check`, Clippy senza warning, 16 test Rust e il flusso browser completo desktop/mobile.

Per riprendere: avviare il servizio, creare l'app Spotify, autorizzare l'account e verificare
raccolta a pagina chiusa, riavvio e sovrapposizione import/API con ascolti reali. I test simulati non
sostituiscono questa verifica. Le funzionalità successive della roadmap e tutte le decisioni aperte
non esplicitamente risolte qui rimangono in sospeso. Nessuna licenza è stata scelta implicitamente.

## 16. Mappa decisionale su GitHub — 20 ottobre 2026

Il primo prototipo locale è pubblicato nel commit
[`cef09db`](https://github.com/itsmrma/scrobbl-rs/commit/cef09db03267fe247606db0b9e6237501d1d305b).
Per pianificare le prossime iterazioni, leggere la
[mappa dal primo prototipo alla prima release](https://github.com/itsmrma/scrobbl-rs/issues/1)
e scegliere fra i suoi sotto-ticket aperti con prerequisiti risolti.

L'utente considera questa versione una base sufficiente come primo prototipo, ma ha segnalato
problemi che descriverà in seguito, anche tramite ticket. Raccogliere prima il feedback e gli
esempi concreti: il comportamento attuale e alcune scelte potranno cambiare. Non presumere
che i test automatici superati risolvano questi problemi o sostituiscano la prova con Spotify reale.

La mappa raccoglie domande, verifiche preliminari e dipendenze; non approva nuove scelte né
avvia automaticamente lo sviluppo. Conservare le conferme di questo documento finché non
vengono riviste esplicitamente. Registrare le nuove decisioni nei rispettivi ticket e aggiornare
questo documento quando cambiano i requisiti; creare poi i ticket di implementazione pertinenti.

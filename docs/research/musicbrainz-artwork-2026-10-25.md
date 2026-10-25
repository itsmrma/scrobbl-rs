# Immagini di album e artisti senza login Spotify

Ricerca del 25 ottobre 2026 per [Quali immagini e garanzie offrono MusicBrainz e le fonti collegate senza login Spotify?](https://github.com/itsmrma/scrobbl-rs/issues/64), nella sotto-mappa [PROBLEMI DA RISOLVERE](https://github.com/itsmrma/scrobbl-rs/issues/60). Fotografia del codice: [7c210e945f3dff88e9fcff703db25d1541135f99](https://github.com/itsmrma/scrobbl-rs/commit/7c210e945f3dff88e9fcff703db25d1541135f99). Nessuna implementazione, modifica dell’archivio personale o prova sul server del proprietario.

## Esito

Il percorso senza account è tecnicamente praticabile per le **copertine**: MusicBrainz identifica un album e Cover Art Archive fornisce le immagini associate a release o release group. Per i **ritratti degli artisti** occorre invece seguire relazioni verso immagini esterne o Wikidata e Wikimedia Commons: non esiste un equivalente endpoint CAA per artista. La copertura non è garantita e i due percorsi non vanno confusi. [API MusicBrainz](https://musicbrainz.org/doc/MusicBrainz_API), [API CAA](https://musicbrainz.org/doc/Cover_Art_Archive/API), [relazione immagine artista](https://musicbrainz.org/relationship/221132e9-e30e-43f2-a741-15afc4c5fa7c), [P18](https://www.wikidata.org/wiki/Property:P18).

La scelta del proprietario è già fissata in [Quale priorità usare per le nuove immagini di album e artisti?](https://github.com/itsmrma/scrobbl-rs/issues/61): nuovo percorso per immagini mancanti, Spotify come fallback, immagini esistenti conservate e immagini personali mai sovrascritte. La ricerca **non** decide ulteriori servizi né autorizza cancellazioni; mette in evidenza due scelte residue, descritte in fondo.

## Fatti documentati: identificazione e copertine

- L’API pubblica MusicBrainz offre JSON e ricerche per nomi; senza MBID si cerca prima l’entità, poi si usa l’identificatore ottenuto. Letture pubbliche ordinarie non richiedono API key o autenticazione; scritture e informazioni dell’utente sì. Le relazioni URL si ottengono includendo `url-rels`. L’accesso non commerciale è gratuito. [API MusicBrainz](https://musicbrainz.org/doc/MusicBrainz_API).
- La ricerca usa Lucene: parametri `query`, `limit` fra 1 e 100 e `offset`; occorre fare escaping della sintassi Lucene **oltre** alla codifica URL. Per album si possono combinare `releasegroup` e `artist`/MBID artista; i risultati hanno punteggi e metadati. Una corrispondenza testuale non dimostra da sola l’identità. [API Search](https://musicbrainz.org/doc/MusicBrainz_API/Search), [campi release group](https://musicbrainz.org/doc/Search_Server), [omonimi e disambiguazione](https://musicbrainz.org/doc/How_to_Add_an_Artist).
- CAA restituisce JSON per `/release/{mbid}` o `/release-group/{mbid}`; il secondo indica anche la release sorgente. Espone front/back, approvazione e miniature 250/500/1200 pixel. Gli endpoint `front-500` ecc. possono rispondere con redirect 307; assenza di immagine/entità può produrre 404 e sovraccarico 503. Una miniatura può non esistere anche dopo il redirect. Le letture documentate non prevedono token. CAA descrive attualmente assenza di regole proprie di rate limiting, ma elenca comunque 503: non è una promessa di capacità illimitata né vale automaticamente per Internet Archive, destinazione dei redirect. [API CAA](https://musicbrainz.org/doc/Cover_Art_Archive/API).

Esempio di ricerca: `releasegroup:"Abbey Road" AND artist:"The Beatles"`. La scelta di una copertina rappresentativa del release group è distinta dall’identificazione dell’**edizione ascoltata**: questo arricchimento non deve riscrivere identità o provenienza degli ascolti. L’ultima frase è un’inferenza applicativa dal [vocabolario del progetto](https://github.com/itsmrma/scrobbl-rs/blob/7c210e945f3dff88e9fcff703db25d1541135f99/GLOSSARY.md).

## Fatti documentati: artisti e fonti collegate

MusicBrainz definisce la relazione Artist–URL `image` (UUID `221132e9-e30e-43f2-a741-15afc4c5fa7c`) e la relazione `wikidata` (UUID `689870a4-a1e4-4912-b17f-7b2664215698`). Sono collegamenti, non immagini archiviate nell’API MusicBrainz; il primo tipo può puntare a host differenti e non conferisce una licenza. [Relazione Image](https://musicbrainz.org/relationship/221132e9-e30e-43f2-a741-15afc4c5fa7c), [relazione Wikidata](https://musicbrainz.org/relationship/689870a4-a1e4-4912-b17f-7b2664215698), [linee guida URL](https://musicbrainz.org/doc/Style/Relationships/URLs).

Un QID noto può essere letto in JSON con `https://www.wikidata.org/wiki/Special:EntityData/Q1299.json`. P18 identifica un file multimediale Commons e può contenere più valori; non garantisce un unico ritratto né la presenza del campo. [Accesso Wikidata](https://www.wikidata.org/wiki/Wikidata:Data_access), [proprietà P18](https://www.wikidata.org/wiki/Property:P18).

L’Action API Commons può risolvere il file e restituire URL, dimensioni, MIME e `extmetadata`; `iiurlwidth` richiede una miniatura e `iiextmetadatafilter` limita i metadati. Questi ultimi possono contenere HTML e richiedono trattamento come contenuto non attendibile prima della visualizzazione. Le richieste di metadati estesi sono costose: poche immagini per richiesta. [API Imageinfo](https://www.mediawiki.org/wiki/API:Imageinfo).

**Inferenza, non scelta approvata:** un percorso ristretto a immagini Commons collegate esplicitamente da MusicBrainz, o ottenute dal QID collegato e da P18, evita lo scraping arbitrario e consente di recuperare l’attribuzione. Restano da scegliere l’ordine fra immagine esplicita e P18 e la politica per più candidati. Non è necessaria una ricerca globale SPARQL per leggere un QID già noto.

## Limiti, attese e persistenza

- **MusicBrainz:** massimo una chiamata al secondo per applicazione; la documentazione del throttling descrive anche un limite medio per IP di una richiesta al secondo e risposte 503. Occorre uno User-Agent con applicazione, versione e contatto. La documentazione sconsiglia il polling frequente di metadati e gli avvii sincronizzati. Il limite riguarda richieste, non immagini completate. [Rate limiting](https://musicbrainz.org/doc/MusicBrainz_API/Rate_Limiting).
- **Wikimedia:** le regole 2026 indicano 200 richieste/minuto per client automatici non autenticati con User-Agent conforme, contro 10 per quelli non identificati; sono sperimentali e modificabili. Concorrenza massima consigliata tre; 429/503 normalmente includono Retry-After. In sua assenza attendere almeno cinque secondi o applicare backoff esponenziale. [Regole aggiornate](https://www.mediawiki.org/wiki/Wikimedia_APIs/Rate_limits).
- **Action API:** preferire richieste seriali, cache di dati riutilizzabili e `maxlag` per i lavori in background. Gli errori API nel JSON vanno controllati, non soltanto lo status HTTP. [Etichetta API](https://www.mediawiki.org/wiki/API:Etiquette).
- **Spotify fallback:** immagini artista richiedono un token OAuth nell’API e hanno URL/dimensioni; ciò non impone un nuovo login per le fonti principali. Le quote sono calcolate su finestre mobili e variano per modalità; un 429 va rispettato con Retry-After. Non è documentata una regola generale “una immagine ogni 60 secondi”. [Get Artist](https://developer.spotify.com/documentation/web-api/reference/get-an-artist), [rate limits Spotify](https://developer.spotify.com/documentation/web-api/concepts/rate-limits).

**Inferenze implementative:** separare limiter e stato di retry per provider, condividere il limiter MusicBrainz tra worker e richieste interattive, mantenere checkpoint e cache dei collegamenti affidabili. Una mancanza confermata può avere un retry differito; timeout, throttling o risposta malformata non dimostrano che l’immagine non esista. Nessuna fonte consultata garantisce la velocità dell’import o il completamento di una libreria.

## Riuso e attribuzione: fatti e vincoli

Il dataset MusicBrainz distingue dati core CC0 e dati supplementari CC BY-NC-SA; le copertine non fanno parte di quel dataset. CAA e Internet Archive ospitano immagini che possono essere protette da copyright: la licenza dei metadati non rende libera l’immagine. [Licenze MusicBrainz](https://musicbrainz.org/doc/About/Data_License), [archivi e copyright](https://musicbrainz.org/doc/Copyright_and_DMCA_Compliance).

Commons accetta immagini libere o di pubblico dominio, ma ogni file ha le proprie condizioni. CC BY richiede attribuzione e riferimento alla licenza; CC BY-SA aggiunge condizioni sulle modifiche. Salvare soltanto byte e MIME perde informazioni necessarie al riuso: servono pagina del file, autore, licenza/URL e informazioni sulle modifiche applicate quando richieste. La permanenza locale è possibile secondo la licenza specifica, non perché un’immagine è semplicemente raggiungibile. [Licenze e riuso Commons](https://commons.wikimedia.org/wiki/Commons:Reusing_content_outside_Wikimedia/licenses/en), [download e riuso](https://commons.wikimedia.org/wiki/Commons:First_steps/Reuse/en).

I termini Spotify, versione 10 del 15 maggio 2025, sezione IV.3, limitano la cache locale di metadati e copertine a quella temporanea necessaria al funzionamento e chiedono di non conservare Spotify Content indefinitamente. Le note Get Artist richiedono inoltre collegamento all’entità Spotify e logo per attribuire immagini e metadati. Questi sono vincoli testuali delle fonti, non una valutazione legale della specifica installazione. [Termini Spotify](https://developer.spotify.com/terms), [note Get Artist](https://developer.spotify.com/documentation/web-api/reference/get-an-artist).

**Conseguenza da decidere:** la permanenza attuale dei byte Spotify è in tensione con questi termini. La ricerca non modifica il contratto del proprietario, non elimina immagini esistenti e non equipara le immagini personali ai download Spotify. Occorre decidere come trattare i nuovi fallback e chiarire il trattamento delle immagini Spotify esistenti prima di cambiare la persistenza.

## Confronto con il codice attuale

Tutti i riferimenti seguenti sono al commit base indicato all’inizio.

| Area | Fatto nel codice | Implicazione per il nuovo percorso |
| --- | --- | --- |
| Accodamento | Solo gruppi di artisti e album; esclusi quelli con immagine o cache negativa valida. [artwork.rs](https://github.com/itsmrma/scrobbl-rs/blob/7c210e945f3dff88e9fcff703db25d1541135f99/src/artwork.rs#L47) | Mantenere la granularità dei gruppi e il controllo dell’immagine esistente. |
| Autenticazione | Worker e recupero freddo controllano spotify_tokens. [artwork.rs](https://github.com/itsmrma/scrobbl-rs/blob/7c210e945f3dff88e9fcff703db25d1541135f99/src/artwork.rs#L14), [library.rs](https://github.com/itsmrma/scrobbl-rs/blob/7c210e945f3dff88e9fcff703db25d1541135f99/src/library.rs#L528) | Il tentativo MusicBrainz/CAA e gli eventuali Wikimedia devono precedere questo vincolo. |
| Cadenza | IMAGE_INTERVAL_MS=60_000 per il background. [artwork.rs](https://github.com/itsmrma/scrobbl-rs/blob/7c210e945f3dff88e9fcff703db25d1541135f99/src/artwork.rs#L12) | Non è un limite documentato di MusicBrainz; disaccoppiare per provider. |
| Persistenza | Byte Spotify riusciti senza scadenza; assenze Spotify scadono dopo un giorno. Custom precede Spotify nelle letture. [library.rs](https://github.com/itsmrma/scrobbl-rs/blob/7c210e945f3dff88e9fcff703db25d1541135f99/src/library.rs#L340), [migrazione 0014](https://github.com/itsmrma/scrobbl-rs/blob/7c210e945f3dff88e9fcff703db25d1541135f99/release-migrations/0014_persistent_spotify_artwork.sql) | Conservare le priorità richieste; risolvere separatamente i vincoli del fallback. |
| Schema | source ammette soltanto spotify/custom; non contiene provenienza/licenza dell’immagine. [migrazione 0009](https://github.com/itsmrma/scrobbl-rs/blob/7c210e945f3dff88e9fcff703db25d1541135f99/release-migrations/0009_artwork.sql) | Servono migrazione e provenienza esplicita, senza cancellare immagini precedenti. |
| Download | HTTPS pubblico, porte/indirizzi controllati, DNS fissato, redirect disabilitati; massimo 5 MiB e PNG/JPEG/WebP. [library.rs](https://github.com/itsmrma/scrobbl-rs/blob/7c210e945f3dff88e9fcff703db25d1541135f99/src/library.rs#L196) | CAA usa redirect: gestire in modo limitato e rivalidare ciascun hop; non disattivare le protezioni. GIF/SVG non sono accettati. |

L’ultima colonna contiene inferenze tecniche, non modifiche eseguite. In particolare, l’automatismo deve rileggere la priorità custom anche prima del salvataggio finale per rispettare un upload avvenuto durante il download.

## Prove pubbliche effettive

Eseguite cinque letture GET seriali, senza cookie o Authorization, con User-Agent identificativo `scrobbl.rs-wayfinder-research/2026.10.25 (https://github.com/itsmrma/scrobbl-rs)`. Fra richieste allo stesso provider è stata mantenuta una pausa di almeno 1,2 secondi dopo la risposta. Nessun download del binario immagine.

| Richiesta | Esito osservato |
| --- | --- |
| MusicBrainz release-group, query Abbey Road + The Beatles, limit 3 | HTTP 200; Abbey Road MBID 9162580e-5df4-32de-80cc-f45a8d8a9b1d, score100; altri candidati score92. [Richiesta](https://musicbrainz.org/ws/2/release-group/?query=releasegroup%3A%22Abbey%20Road%22%20AND%20artist%3A%22The%20Beatles%22&fmt=json&limit=3). |
| MusicBrainz artista Beatles, inc=url-rels | HTTP 200; relazioni image verso File:The_Fabs.JPG su Commons e wikidata verso Q1299. [Richiesta](https://musicbrainz.org/ws/2/artist/b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d?inc=url-rels&fmt=json). |
| CAA release-group Year Zero | HTTP 200 dopo redirect a Internet Archive; front approvato e miniature250/500/1200; campo image con URL HTTP legacy. Non seguire automaticamente un downgrade HTTP. [Richiesta](https://coverartarchive.org/release-group/3bd76d40-7f0e-36b7-9348-91a33afee20e). |
| Wikidata Q1299 JSON | HTTP 200; tre valori P18, quindi “prendere il primo” richiede una politica. [Richiesta](https://www.wikidata.org/wiki/Special:EntityData/Q1299.json). |
| Commons imageinfo per File:The_Fabs.JPG, larghezza500, metadati selezionati | HTTP 200; file normalizzato con descriptionurl, miniature HTTPS, MIME JPEG, autore/crediti e Public domain. extmetadata contiene HTML. [Pagina sorgente risolta](https://commons.wikimedia.org/wiki/File:The_Beatles_members_at_New_York_City_in_1964.jpg), [contratto della query](https://www.mediawiki.org/wiki/API:Imageinfo). |

Questi esempi provano la possibilità del percorso pubblico per casi noti. Non misurano qualità/copertura della libreria personale, velocità complessiva, disponibilità futura o correttezza di un nuovo adapter, che non è stato implementato. Alcune letture delle pagine di documentazione attraverso il browser di ricerca hanno restituito429; la documentazione è stata consultata anche attraverso risultati indicizzati primari, e i cinque GET applicativi sopra sono operazioni distinte riuscite.

## Scelte residue e seguito

Entrambi i punti seguenti sono tracciati nella decisione ancora aperta [Come completare le fonti delle immagini e la conservazione del fallback Spotify?](https://github.com/itsmrma/scrobbl-rs/issues/68).

1. **Fonti artista e ambiguità.** Autorizzare immagini Commons collegate da MusicBrainz e Wikidata P18, con provenienza/attribuzione, oppure restringere ulteriormente il recupero? Come scegliere se esistono più immagini o omonimi? Raccomandazione da discutere: percorso Commons ristretto, preferenza per un collegamento immagine esplicito, niente scraping arbitrario, nessuna associazione automatica quando l’identità non è affidabile; Spotify disponibile come ripiego e altrimenti immagine assente.
2. **Fallback Spotify e conservazione.** Come conciliare il requisito di permanenza con i termini del provider? Alternative da discutere: nuovi fallback in cache temporanea con attribuzione corretta, oppure assenza di nuovi download Spotify automatici e uso delle fonti riutilizzabili/custom. Il trattamento dei dati esistenti richiede una scelta esplicita: questa ricerca non ne autorizza la rimozione.

Dopo queste decisioni il ticket di implementazione può definire gli adapter, la migrazione non distruttiva e prove isolate per immagine già presente, upload concorrente, assenza di login Spotify, match ambiguo, assenza confermata, timeout/throttling, redirect e ripresa. Il rapporto resta su ramo research/*, non destinato al merge.

# Collegamento Spotify su HTTP privato — prototipo eliminabile

Asset per [Come rendere semplice il collegamento Spotify da PC/Mac su un server privato HTTP?](https://github.com/itsmrma/scrobbl-rs/issues/17).

**Stato aggiornato dopo il feedback: proposta SSH esclusa dal percorso da sviluppare.** Il proprietario preferisce una finestra guidata, considera la prima demo troppo complicata e vuole evitare anche l'abilitazione di SSH su TrueNAS. La preferenza grafica non approva i prerequisiti SSH. Il meccanismo senza SSH resta da individuare e verificare: non è già stata approvata un'alternativa HTTPS, né il trasferimento di dati OAuth sul normale HTTP LAN. La decisione resta aperta nel ticket.

Il resto di questo documento e l'HTML conservano la proposta precedente come evidenza storica, non come soluzione consigliata. Questo branch non è destinato al merge. Non contiene credenziali né modifica il backend o la distribuzione.

Aprire con un doppio clic [`static/spotify-connect.prototype.html`](../../static/spotify-connect.prototype.html). È un singolo file autonomo, senza dipendenze o connessioni di rete automatiche. Le interazioni simulano il comportamento in memoria; anche salvataggio, rinnovo e riavvio sono simulati.

## Domanda da risolvere

È sufficientemente semplice usare un tunnel SSH temporaneo da PC/Mac per prima autorizzazione e riconnessioni, oppure occorre un helper grafico? Il percorso con HTTPS già disponibile è mostrato per confronto e non richiede appoggio locale.

Il prototipo offre nove percorsi guidati e azioni libere: primo collegamento, annullamento e riprova, callback mancante/scaduta, sessione errata, tunnel interrotto, riavvio, riconnessione, porta occupata e fallimento dello scambio del codice. Tutta l'interfaccia proposta è in inglese.

## Proposta tecnica da validare

1. Dalle Settings sull'indirizzo privato, aprire la guida PC/Mac senza avviare ancora OAuth né trasferire sessioni.
2. Se HTTP, aprire un tunnel SSH verso il server verificandone l'identità. Il tunnel trasporta **l'intera interfaccia di setup**, non soltanto la callback. Il listener locale è limitato a `127.0.0.1`.
3. Aprire `http://127.0.0.1:3210` e autenticarsi qui a scrobbl.rs, nello stesso browser/profilo che farà l'autorizzazione. Questo crea la sessione corretta sull'origine locale; la sessione LAN non viene copiata.
4. Solo al primo setup, creare/configurare l'app nel Developer Dashboard, registrare `http://127.0.0.1:3210/auth/spotify/callback`, inserire Client ID/Secret nella pagina raggiunta tramite tunnel e verificare l'accesso dell'account. Questi passaggi non sono implementati dal file dimostrativo.
5. Premere Connect Spotify da questa origine. Il server crea state monouso legato alla sessione locale con scadenza di dieci minuti. Spotify torna alla medesima origine; il tunnel inoltra la richiesta allo stesso backend, che verifica sessione e state e scambia il codice direttamente via HTTPS.
6. Dopo lo scambio e la persistenza sul server, mostrare Connected. Solo allora chiudere il browser e interrompere il tunnel. Raccolta e refresh ordinario restano processi del server. Per consultare da iPhone/iPad usare l'indirizzo privato abituale.

Esempio illustrativo, non eseguito e non già pronto per il server del proprietario:

```sh
ssh -N -o ExitOnForwardFailure=yes -L 127.0.0.1:3210:127.0.0.1:3210 YOUR_USER@YOUR_SERVER
```

SSH e permesso di forwarding sul server sono **nuovi prerequisiti proposti**, non requisiti già accettati. L'helper grafico mostrato è un possibile involucro dello stesso trasporto SSH, non un programma esistente e non una soluzione priva di questo prerequisito. Nessun requisito di SSH aperto su Internet: la raggiungibilità resta LAN/VPN. Non disabilitare la verifica della chiave host.

Il secondo `127.0.0.1` è visto dall'host SSH: in Docker/Portainer occorre un endpoint adeguato sull'host. Non si assume che il loopback del container sia automaticamente quello di TrueNAS. L'esempio presuppone destinazione sullo stesso host; una destinazione HTTP su una macchina diversa aggiungerebbe un tratto non cifrato.

## Fatti verificati nel codice, non prove di esercizio

Baseline locale `d7b7a92`:

- `src/config.rs:58–65`: bind, origine e redirect sono derivati da `127.0.0.1:SCROBBL_PORT`.
- `src/web.rs:75–98`: Host esatto per tutte le richieste; Origin esatta per i metodi di scrittura. Con la baseline la porta locale deve corrispondere a SCROBBL_PORT. Cambiare soltanto la porta locale o l'host della callback non funziona.
- `src/auth.rs:85–90`: cookie host-only, HttpOnly, SameSite=Lax; il login sull'IP LAN non equivale al login su loopback.
- `src/spotify.rs:140–158,178–227`: state associato alla sessione, monouso, dieci minuti; consumato prima dello scambio. Se lo scambio fallisce occorre un nuovo tentativo.
- `src/spotify.rs:255,310`: token persistiti in SQLite sul server e rinnovo sul server; nessuna cifratura applicativa dei token a riposo. I segreti inseriti nel modulo passano dal browser; non significa che il browser non li veda mai.
- `static/app.js:50,592`: API relative all'origine e navigazione nella finestra corrente, coerenti con l'idea di inoltrare l'intera applicazione.
- `src/main.rs:8`, `src/spotify.rs:584`: raccolta sul server indipendente da browser e tunnel.

La lettura rende il percorso plausibile con l'attuale origine loopback; **non è stata eseguita un'autorizzazione reale né una prova SSH verso TrueNAS**. La baseline non offre ancora accesso quotidiano LAN/VPN o HTTPS: questa proposta non completa il supporto multi-origine. Bind, origini ammesse e redirect devono diventare configurazioni esplicite senza eliminare Host/Origin/CSRF. L'origine e il redirect di ciascun tentativo dovranno restare coerenti; per HTTPS occorre verificare anche cookie Secure e proxy fidati.

Il modello propone annullamento esplicito e invalidazione dei tentativi incompleti al riavvio. Queste scelte di recupero richiedono conferma; non descrivono una funzionalità già verificata della baseline. Una riconnessione non cancella l'archivio. Non introdurre cambiamenti impliciti del comportamento di raccolta se un vecchio collegamento è ancora valido.

## Trasporto e alternative

- Password di setup, sessione locale, Client Secret inserito e codice OAuth attraverserebbero loopback + tunnel SSH. Scambio e rinnovo dei token vanno dal server a Spotify via HTTPS; i token non vengono copiati a mano. La consultazione HTTP quotidiana mantiene il compromesso LAN già discusso nella decisione d'installazione: il tunnel temporaneo non cifra quel traffico successivo.
- Con HTTPS già disponibile, avviare e concludere OAuth direttamente nell'origine HTTPS autenticata. Configurare esattamente il redirect della stessa origine. Questo prototipo non decide come procurarsi dominio/certificati.
- Un helper che inoltri semplicemente callback o token all'IP HTTP non è una soluzione equivalente verificata: restano associazione di sessione e trasporto sensibile. Non proporre criptografia ad hoc o copia di cookie/token/database come scorciatoia.
- Multi-scrobbler documenta alternative locali, inclusi copia della configurazione e cambio manuale dell'host; sono precedenti consultati, non percorsi adottati per scrobbl.rs.

## Prove richieste prima della distribuzione privata

Da integrare nella decisione finale e collegare a [Validare Spotify reale: OAuth, raccolta a pagina chiusa e riconciliazione dello storico](https://github.com/itsmrma/scrobbl-rs/issues/3), dopo aver scelto con il proprietario il percorso:

- Windows/macOS + Firefox: disponibilità SSH, accesso permesso, chiave host verificata, inoltro effettivo, errore per porta occupata, endpoint TrueNAS/Portainer raggiungibile; nessun listener locale esposto alla LAN.
- App/account reale: redirect esatto accettato; sessione loopback autenticata prima di Connect; ritorno nel medesimo profilo; callback con sessione errata, state scaduto o riusato rifiutata senza sostituire credenziali.
- Annullamento, callback mancante, tunnel interrotto, scambio fallito e riavvio durante la procedura: ripresa comprensibile senza riutilizzare un codice consumato né cancellare l'archivio.
- Dopo Connected: token nel volume persistente, refresh ordinario e raccolta a browser/tunnel chiusi; conservazione e ripresa dopo riavvio e ricreazione del container. Nessun segreto in log o allegati pubblici.
- Errore transitorio: retry/backoff sul server. Refresh token rifiutato come invalido: stato Reconnect required e nuova autorizzazione guidata. Non promettere una durata garantita per ogni connessione né che ogni errore significhi scadenza.
- Percorso HTTPS con callback diretta, cookie e controlli dietro proxy; consultazione Safari su iPhone/iPad indipendente dal PC/Mac.

## Fonti ricontrollate il 20 ottobre 2026

- [Spotify Redirect URIs](https://developer.spotify.com/documentation/web-api/concepts/redirect_uri): HTTPS o eccezione HTTP per IP loopback esplicito, non `localhost` né IP LAN. È documentata anche la registrazione loopback senza porta per porte dinamiche; la baseline del progetto non supporta automaticamente questa flessibilità.
- [Spotify Refreshing tokens](https://developer.spotify.com/documentation/web-api/tutorials/refreshing-tokens#refresh-token-lifetime): per app del Developer Dashboard è documentata una durata di sei mesi del refresh token, non estesa dai rinnovi ordinari; una nuova autorizzazione avvia un nuovo periodo. Riconfermare durante il collaudo reale.
- [OpenSSH ssh(1)](https://man.openbsd.org/ssh.1): `-L` inoltra il listener locale sul canale SSH; `-N` non esegue comandi remoti; verifica dell'identità del server mediante host key.
- [Multi-scrobbler Spotify](https://docs.multi-scrobbler.app/configuration/sources/spotify/): precedenti locali da non assumere compatibili con il contratto di sessione di scrobbl.rs.

## Stato della verifica dell'asset

Verifica sintattica dei due script inline con Node. Nessuna nuova suite di test: il file è un prototipo eliminabile. Il controllo visuale/interattivo tramite il browser della sessione non è stato possibile perché l'inventario Computer Use restituisce zero browser e zero app. Occorre quindi ancora aprirlo e provarlo con il proprietario; la decisione resta aperta.

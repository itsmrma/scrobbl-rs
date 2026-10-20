# Avvio locale

Le nuove fondamenta richiedono Rust 1.94 o successivo e una directory dati nuova rispetto al
prototipo. L'avvio rifiuta `scrobbl.db` senza aprirlo. Su PowerShell, per esempio:

```powershell
$env:SCROBBL_DATA_DIR = ".\data-foundations"
cargo run --locked
```

Reimporta lo storico nella nuova installazione. Archivio musicale e segreti sono separati in
`archive.db` e `private/auth.db`; conserva privata l'intera directory. I dettagli tecnici sono in
[archive-foundations.md](archive-foundations.md).

Dalla cartella del progetto:

```sh
./scripts/run-local.sh
```

Apri **http://127.0.0.1:3210**. Crea la password del proprietario, di almeno 12 caratteri.
Non serve Docker. Il processo Rust deve restare avviato per raccogliere gli ascolti; puoi chiudere
la pagina. Per arrestarlo usa Ctrl+C, poi riavvialo con lo stesso comando. I dati restano in `data/`.

## Collegare Spotify

1. Apri **Settings** nell'app: contiene la procedura e il callback da copiare.
2. Vai su <https://developer.spotify.com/dashboard> con il tuo account Premium e crea un'app
   chiamata **scrobbl.rs**, selezionando **Web API** se richiesto.
3. Registra il callback esatto `http://127.0.0.1:3210/auth/spotify/callback`.
4. Salva e copia **Client ID** e **Client Secret** nel modulo locale di scrobbl.rs. Non inviare il
   segreto in chat. Se l'account di ascolto è diverso da quello che crea l'app, aggiungilo anche
   in **Users Management**.
5. Premi **Save Spotify app**, poi **Connect Spotify** e autorizza l'accesso nel browser.
6. Controlla **Connected** e l'ultima sincronizzazione riuscita; riproduci musica e verifica
   **Overview** e **Listening history**.

La pagina e il server devono essere aperti sullo stesso computer per questo callback locale.
Se cambi porta tramite `.env`, registra in Spotify il nuovo callback mostrato dall'app.
Il rinnovo dell'autorizzazione è automatico. La guida completa e le fonti ufficiali sono in
[spotify-setup.md](spotify-setup.md).

## Importare lo storico

Richiedi **Extended streaming history** nelle impostazioni privacy di Spotify. Estrai lo ZIP e,
alla pagina **Import history**, seleziona uno o più JSON musicali. Massimo 64 MiB per caricamento:
per archivi grandi usa più caricamenti. I file già completati restano salvati se un file successivo
fallisce; puoi ripetere il caricamento senza duplicarli.

Gli ascolti importati sotto 30 secondi restano nella cronologia e non contano nelle statistiche.
Le durate dalle API sono stimate; l'import conserva la durata effettiva. Le corrispondenze sicure
entro 2 secondi sullo stesso URI Spotify vengono unite, conservando entrambe le fonti. Il comportamento
sui timestamp del tuo account va ancora verificato con dati reali.

La prova conclusiva è: collegare Spotify, ascoltare con la pagina chiusa, riaprire e controllare gli
ascolti, riavviare il servizio e verificare che dati e autorizzazione persistano. I test automatici
usano archivi temporanei e risposte Spotify simulate, senza toccare il tuo account.

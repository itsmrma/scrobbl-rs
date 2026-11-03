# Schermata di avvio immediato, transizioni di riavvio/spegnimento e design system

L'evoluzione dell'interfaccia verso un archivio musicale fruibile su schermi desktop e mobili (ADR 0004) ha evidenziato diverse frizioni: l'irraggiungibilità dell'istanza durante l'avvio del container Docker, la mancanza di feedback a tutto schermo durante le operazioni di arresto e riavvio dell'applicazione, l'eccessiva annidazione dei form di connessione (in particolare Spotify) e la presenza di controlli visivamente invasivi o privi di spaziatura coerente. Per risolvere questi aspetti in modo durevole e uniforme, vengono deliberate le seguenti scelte architetturali e di design:

1. **Early-binding HTTP listener e Schermate di stato dell'applicazione (Avvio, Riavvio, Spegnimento):**
   Il listener HTTP del server si collega alla porta configurata immediatamente all'avvio del processo, prima o contestualmente all'inizializzazione del pool SQLite e all'esecuzione delle migrazioni del database. Uno stato di ciclo di vita (`BootState`) gestisce le transizioni:
   - Durante l'avvio (`BootState::Initializing`), il server risponde immediatamente alle richieste browser servendo una schermata di caricamento autonoma a zero dipendenze esterne, dotata di polling leggero (`/api/boot-status`) e indicatori di avanzamento e gestione errori.
   - Le richieste API ricevono uno stato `503 Service Unavailable` controllato con `Retry-After: 1`.
   - Al completamento dell'avvio (`BootState::Ready`), il traffico viene instradato all'applicazione React completa. Lo stesso guscio visivo di avvio è integrato in `index.html` per garantire continuità prima dell'idratazione client.
   - Nelle operazioni di **Riavvio** e **Spegnimento** da interfaccia, l'app passa a una schermata di transizione a schermo intero coordinata nello stesso linguaggio visivo: conferma la chiusura in sicurezza dell'archivio, esegue il polling verso il nuovo processo riavviato ricaricando automaticamente l'app, oppure fornisce un chiaro messaggio di arresto confermato con pulsante per riverificare la connessione.

2. **Appiattimento delle Impostazioni e Scheda di stato del collegamento:**
   La schermata Settings mantiene la suddivisione primaria in 8 sezioni logiche, ma ne azzera la profondità di annidamento interno:
   - La sezione Connessioni (Spotify) separa tassativamente la consultazione operativa dal primo setup. Quando il servizio è connesso, presenta una scheda piatta con badge di stato, azioni immediate (pausa, sincronizzazione manuale, disconnessione) e intervalli di polling, eliminando guide e accordion aperti.
   - Le istruzioni tecniche su reverse proxy, DNS e certificati HTTPS vengono confinate in una finestra di dialogo modale accessibile solo su richiesta esplicita.
   - I provider di immagini presentano una lista piatta con toggle diretti e informazioni di licenza in popover/dialog.
   - L'azione di ripristino delle copertine mancanti ("Refresh missing artwork") viene rimossa dai Settings e collocata nei contesti operativi naturali: nella card del worker in Attività e nel menu strumenti della Libreria.

3. **Uniformazione dei controlli e sistema di spaziatura:**
   - Tutti i selettori di vista e filtri adottano il modello del "Controllo a segmenti" (Segmented Control): contenitore a contrasto neutro incassato, elemento attivo a sfondo solido a contrasto, eliminazione di qualsiasi contorno fluorescente o glow verde neon, riservando l'accento unicamente a micro-dettagli discreti conformemente alla *Rarity Rule*.
   - Le spaziature applicano token rigidi e coerenti: container centrato su desktop con larghezza massima protetta per prevenire vuoti asimmetrici, e layout a colonna piena su mobile con padding laterale e gap uniformi a 14px e 12px.
   - Le liste di riproduzione, ranking e storico adottano righe con micro-distacco verticale (4px) e sfondi/hover definiti per garantire una scansione visiva netta.
   - Le transizioni e i cambi vista adottano micro-animazioni snelle (150–180ms), disattivabili via `prefers-reduced-motion`.

Riferimento: [Issue #156](https://github.com/itsmrma/scrobbl-rs/issues/156), [Spec #158](https://github.com/itsmrma/scrobbl-rs/issues/158).

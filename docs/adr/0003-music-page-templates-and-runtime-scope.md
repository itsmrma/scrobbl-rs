# Template delle pagine musicali e ambito dei dati a runtime

La modularità della Home descritta nell'ADR 0002 viene estesa alle pagine artista, album e brano attraverso un unico motore di griglia e configurazione. Ogni tipo di pagina musicale ha un template condiviso: la disposizione e le opzioni dei widget appartengono al template, mentre l'identità dell'entità visualizzata appartiene al contesto runtime (`Global`, `Artist(id)`, `Album(id)`, `Track(id)`) e non viene salvata nel template. Si evita così di duplicare la griglia o di introdurre configurazioni per singola identità musicale.

Un registro dei widget esplicita gli ambiti compatibili, il supporto ai periodi e i dati necessari. Le richieste riutilizzano la selezione condivisa dell'archivio, preservando raggruppamenti, correzioni locali e regole di consultazione esistenti. L'header dell'entità rimane esterno alla griglia personalizzabile; i contenuti analitici appartengono alla griglia. Questa separazione conserva identità, navigazione e azioni anche quando l'utente nasconde o riordina i widget.

La configurazione usa un solo editor contestuale, accessibile dalla pagina musicale e dalle impostazioni. L'entità aperta fornisce l'anteprima; l'editor rende esplicito che il salvataggio aggiorna tutte le pagine di quel tipo e mantiene salvataggio, annullamento e ripristino coerenti con la Home.

La Home e i tre template sono configurazioni indipendenti su SQLite, ciascuna con revisione e controllo ottimistico dei conflitti. La configurazione Home già salvata viene preservata. Il contesto di consultazione e i filtri temporanei non modificano il template: il salvataggio resta esplicito e un conflitto non deve cancellare la bozza. Le altre sessioni rileggono la configurazione all'apertura della pagina o al ritorno in primo piano.

Confermato dal proprietario il 30 ottobre 2026, inclusa la sintesi finale della sessione. La decisione estende l'ADR 0002. Cataloghi, indicatori, default e criteri di verifica sono definiti nella [specifica #127](https://github.com/itsmrma/scrobbl-rs/issues/127), pronta per l'implementazione.

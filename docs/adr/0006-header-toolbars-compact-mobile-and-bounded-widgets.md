# Intestazioni a toolbar unica, metriche compatte su mobile e widget a altezza limitata

La revisione grafica di [#189](https://github.com/itsmrma/scrobbl-rs/issues/189) mantiene il linguaggio visivo (token, tinting OKLab, punto d'accento dopo i titoli di pagina, artista, album e brano) ma corregge disposizioni che lasciavano spazio inutilizzato o controlli isolati. Questa decisione supera due punti di ADR 0004 e precisa il layout predefinito di ADR 0002.

1. **Intestazioni a toolbar unica.** Le intestazioni di pagina e delle pagine musicali hanno titolo e sottotitolo a sinistra e un'unica riga allineata di azioni e filtri (Periodo, Ordina per). La navigazione (Indietro) sta sopra il titolo e non flotta mai da sola. Su schermi ≤ 600px i pulsanti di un'intestazione condividono la riga con etichette brevi (il nome accessibile resta quello completo) invece di espandersi ciascuno al 100% della larghezza: questo supera la regola "espandendosi all'intera larghezza" di ADR 0004.

2. **Metriche compatte nei widget stretti.** Quando il widget Totals è largo al massimo 480px (telefono, oppure span 2 su desktop), le tre metriche restano affiancate con valore a 22px, sempre direttamente sotto l'etichetta e allineate a sinistra. La regola resta una container query sul widget, come richiesto da ADR 0004; cambia solo la disposizione, che sostituisce la colonna singola a 28px e occupa circa un terzo dell'altezza.

3. **Widget affiancati ad altezza confrontabile.** In una riga a più colonne gli elenchi lunghi hanno altezza limitata con scorrimento interno e terminano su righe intere; un widget corto non viene stirato accanto a uno lungo.

4. **Now Playing fuori dal layout predefinito.** Il layout di fabbrica della Overview (ADR 0002) non include più il widget Now Playing, perché la scheda nella barra laterale mostra già la riproduzione. Il widget resta nel catalogo; i layout salvati non cambiano.

Riferimento: [Ticket #189](https://github.com/itsmrma/scrobbl-rs/issues/189), ADR 0002, ADR 0004.

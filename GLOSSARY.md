# scrobbl.rs

Vocabolario dell'archivio personale di ascolti e della pulizia della libreria.

## Language

**Ascolto**:
Un evento in cui è stata ascoltata musica. Ripetere lo stesso brano produce ascolti distinti.

**Tempo ascoltato effettivo**:
Il tempo che una fonte riporta come effettivamente riprodotto in un ascolto, distinto dalla durata totale del brano.

**Tempo ascoltato stimato**:
Una valutazione del tempo riprodotto ricavata dalle informazioni disponibili, anziché da un tempo effettivo riportato dalla fonte. Non dimostra che il brano sia stato ascoltato per intero.

**Tempo ascoltato sconosciuto**:
Il tempo riprodotto di un ascolto per il quale non sono disponibili né un valore effettivo né una stima. L'assenza di questa informazione non equivale a un tempo di zero secondi.

**Osservazione originale**:
Il resoconto di un ascolto ricevuto da una fonte, con i metadati e i tempi forniti da quella fonte.

**Riconciliazione di ascolti**:
L'associazione delle osservazioni originali che descrivono lo stesso ascolto, rappresentato una sola volta nell'archivio. È distinta dall'unione di identità musicali e dal raggruppamento di edizioni.

**Variante originale**:
Una versione dell'osservazione originale che differisce nei dati ricevuti dalla fonte pur riferendosi allo stesso ascolto.

**Occorrenza sorgente**:
La presenza di una riga in una specifica posizione di un file importato. Più occorrenze possono riportare la stessa osservazione originale.

**Edizione ascoltata**:
La specifica pubblicazione del brano associata a un ascolto, per esempio sull'album normale o sulla deluxe.

**Raggruppamento di brani**:
L'associazione di brani di edizioni diverse per consultarne insieme gli ascolti, conservando l'edizione ascoltata.
_Avoid_: Duplicato di ascolto.

**Raggruppamento di album**:
L'associazione di edizioni di un album per consultarne insieme gli ascolti. È una scelta distinta dal raggruppamento dei brani che contengono.

**Unione di identità**:
L'accorpamento di schede che rappresentano lo stesso artista, album o brano nella libreria.
_Avoid_: Raggruppamento di edizioni, deduplicazione degli ascolti.

**Identità provvisoria**:
Un artista, album o brano identificato con informazioni incomplete, in attesa di un collegamento certo o confermato a un'identità riconosciuta.

**Correzione locale**:
Una modifica dell'utente ai metadati della propria libreria, distinta dai valori ricevuti dalle fonti.

**Regola di pulizia**:
Un criterio scelto dall'utente per uniformare metadati o interpretare nomi e featuring nei dati importati.

**Cestino**:
L'insieme degli ascolti cancellati che possono ancora essere recuperati prima dello svuotamento esplicito.

**Calendario assegnato**:
Il fuso orario e il giorno di inizio settimana usati per collocare un ascolto nei giorni e nelle settimane della consultazione. È distinto dall’istante originale dell’ascolto e può differire fra ascolti acquisiti con impostazioni diverse.

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
Il resoconto di un ascolto ricevuto da una fonte, con i metadati e i tempi forniti da quella fonte. È distinto dal file e dai dati grezzi che lo contengono.

**Conservazione compatta**:
La conservazione dei dati normalizzati degli ascolti, delle impronte e della provenienza, senza i dati grezzi delle righe. I file sorgente restano disponibili finché servono a completare o riprendere un import.

**Riconciliazione di ascolti**:
L'associazione delle osservazioni originali che descrivono lo stesso ascolto, rappresentato una sola volta nell'archivio. È distinta dall'unione di identità musicali e dal raggruppamento di edizioni.

**Possibile duplicato di ascolto**:
Un record per cui le informazioni disponibili non bastano a stabilire se descriva un ascolto già presente. I record restano separati fino a una revisione esplicita, individuale o automatica su richiesta dell'utente, che può mantenerli distinti oppure conservarne uno e spostare gli altri nel cestino con possibilità di annullamento.

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

**Voto personale**:
La valutazione dell'utente per un artista, un album o un brano, da mezza a cinque stelle con passi di mezza stella. È distinta dal numero degli ascolti e dalla posizione in classifica; l'assenza di voto non equivale a zero stelle.

**Regola di pulizia**:
Un criterio scelto dall'utente per uniformare metadati o interpretare nomi e featuring nei dati importati.

**Cestino**:
L'insieme degli ascolti cancellati che possono ancora essere recuperati prima dello svuotamento esplicito.

**Calendario assegnato**:
Il fuso orario e il giorno di inizio settimana usati per collocare un ascolto nei giorni e nelle settimane della consultazione. È distinto dall’istante originale dell’ascolto e può differire fra ascolti acquisiti con impostazioni diverse.

**Raggruppamento per nome**:
La consultazione congiunta di schede artista con lo stesso nome, o album con lo stesso titolo e artista. Non dimostra che le identità originali coincidano e può essere modificata senza perdere le schede sorgente.

**Separazione dal gruppo**:
La scelta di consultare separatamente una o più schede prima raggruppate per nome, per distinguere per esempio degli omonimi. È reversibile e non cancella ascolti o informazioni originali.
_Avoid_: Decoupling, cancellazione di duplicati.

**Template di pagina musicale**:
La disposizione personalizzabile dei widget condivisa da tutte le pagine di uno stesso tipo: artista, album o brano. L'utente dispone di un template per ciascun tipo; modificarlo cambia la disposizione per tutte le pagine di quel tipo.
_Avoid_: Layout del singolo artista, layout del singolo album, layout del singolo brano.

**Calendario degli ascolti**:
La rappresentazione giorno per giorno degli ascolti conteggiati di un artista, album o brano, secondo il calendario assegnato. L'intensità del colore esprime il numero degli ascolti.

**Traguardo di ascolto**:
Un evento significativo nello storico degli ascolti conteggiati di un artista, album o brano, come il primo ascolto o il raggiungimento di una soglia di ascolti. Il riferimento è l'intero storico dell'entità, indipendentemente dal periodo consultato.

**Serie di giorni di ascolto**:
Una successione di giorni consecutivi, secondo il calendario assegnato, con almeno un ascolto conteggiato dello stesso artista, album o brano per giorno. La serie in corso termina oggi o ieri; la serie più lunga può appartenere a qualsiasi periodo dello storico.
_Avoid_: Streak senza indicazione dell'entità ascoltata.

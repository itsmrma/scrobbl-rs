// Simulates the exact frontend queueArtwork + backend artwork_fetches interaction
// to verify the on-the-fly failure rate when browsing pages.

class ArtworkRetry extends Error {
  constructor(delayMs) {
    super("Artwork temporarily unavailable");
    this.delayMs = delayMs;
  }
}

// Simulated backend semaphore with capacity 1 and download duration
class MockServer {
  constructor(downloadDurationMs = 3500) {
    this.inFlight = false;
    this.downloadDurationMs = downloadDurationMs;
  }

  async fetchArtwork(id) {
    if (this.inFlight) {
      // 503 Service Unavailable, Retry-After: 1
      throw new ArtworkRetry(1000);
    }
    this.inFlight = true;
    try {
      await new Promise((r) => setTimeout(r, this.downloadDurationMs));
      return { id, data: Buffer.from("image_data") };
    } finally {
      this.inFlight = false;
    }
  }
}

// Client queueArtwork implementation from frontend/src/artwork-activity.ts
class ClientQueue {
  constructor(server) {
    this.server = server;
    this.queue = [];
    this.busy = false;
    this.stats = { total: 0, loaded: 0, failed: 0 };
  }

  async pump() {
    if (this.busy) return;
    this.busy = true;
    try {
      while (this.queue.length) await this.queue.shift()();
    } finally {
      this.busy = false;
    }
  }

  queueArtwork(id) {
    this.stats.total++;
    let attempts = 0;

    return new Promise((resolve, reject) => {
      const run = async () => {
        let loaded = false;
        let retry = false;
        try {
          const res = await this.server.fetchArtwork(id);
          loaded = true;
          this.stats.loaded++;
          resolve(res);
        } catch (error) {
          if (error instanceof ArtworkRetry && attempts++ < 3) {
            retry = true;
            setTimeout(() => {
              this.queue.push(run);
              void this.pump();
            }, error.delayMs);
          } else {
            this.stats.failed++;
            reject(error);
          }
        }
      };

      this.queue.push(run);
      void this.pump();
    });
  }
}

async function runSimulation() {
  console.log("=== Simulazione Download al Volo (On-the-fly browsing) ===");
  const server = new MockServer(3500); // realistic single image lookup time (MB sleep 1.1s + API + download)
  const client = new ClientQueue(server);

  // User opens a page with 6 images (e.g. top albums grid)
  const imageIds = ["album-1", "album-2", "album-3", "album-4", "album-5", "album-6"];
  console.log(`Caricamento pagina con ${imageIds.length} immagini contemporanee...`);

  const results = await Promise.allSettled(imageIds.map((id) => client.queueArtwork(id)));

  console.log("\nEsiti download per singola immagine:");
  results.forEach((r, idx) => {
    if (r.status === "fulfilled") {
      console.log(`  - [OK] ${imageIds[idx]}: Caricata con successo`);
    } else {
      console.log(`  - [FALLITA] ${imageIds[idx]}: ${r.reason.message} (superati tentativi di retry)`);
    }
  });

  console.log("\nStatistiche finali:");
  console.log(`  Totale richieste: ${client.stats.total}`);
  console.log(`  Caricate: ${client.stats.loaded} (${Math.round((client.stats.loaded / client.stats.total) * 100)}%)`);
  console.log(`  Fallite: ${client.stats.failed} (${Math.round((client.stats.failed / client.stats.total) * 100)}%)`);

  // Red-capable assertion
  if (client.stats.failed > 0) {
    console.error(`\n[ROSSO]: ${client.stats.failed} immagini su ${client.stats.total} sono fallite durante la navigazione!`);
    process.exit(1);
  }
}

runSimulation();

import { useEffect, useState } from "react";
import { api } from "./api";
import type { ApiKeySettings, Preferences } from "./generated";
import { date, ErrorText, useToast } from "./ui";

export function ApiKeys({ preferences }: { preferences: Preferences }) {
  const [settings, setSettings] = useState<ApiKeySettings | null>(null);
  const [name, setName] = useState("");
  const [application, setApplication] = useState("spotify_analysis");
  const [expiry, setExpiry] = useState("");
  const [origins, setOrigins] = useState("");
  const [secret, setSecret] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const notify = useToast();
  useEffect(() => {
    let cancelled = false;
    api
      .apiKeys()
      .then((data) => {
        if (!cancelled) {
          setSettings(data);
          setOrigins(data.cors_origins.join("\n"));
        }
      })
      .catch((e) => {
        if (!cancelled) setError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, []);
  async function action(work: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await work();
      setSettings(await api.apiKeys());
    } catch (e) {
      setError(e instanceof Error ? e.message : "The request failed.");
    } finally {
      setBusy(false);
    }
  }
  return (
    <article className="panel padded api-keys-panel">
      <h2>API keys</h2>
      <p>
        Read-only access to your listening history. Keys cannot change settings,
        import data, access backups or manage Spotify.
      </p>
      <ErrorText>{error}</ErrorText>
      {settings && (
        <>
          <h3>Connect an application</h3>
          <label>
            API base URL
            <input readOnly value={settings.base_url} />
          </label>
          <button
            className="secondary"
            disabled={busy}
            onClick={() =>
              void action(async () => {
                await navigator.clipboard.writeText(settings.base_url);
                notify("API URL copied.");
              })
            }
          >
            Copy API URL
          </button>
          {!settings.base_url.startsWith("https://") && (
            <p className="warning">
              This instance uses HTTP. Configure the external HTTPS origin
              before connecting from GitHub Pages. HTTP loopback is for local
              development.
            </p>
          )}
          <p>
            Use your private HTTPS address on LAN or VPN. Create a key, then
            copy the URL and secret into an application that supports the
            scrobbl.rs read API.
          </p>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void action(async () => {
                const created = await api.createApiKey({
                  name,
                  application,
                  expires_at_ms: expiry ? new Date(expiry).getTime() : null,
                });
                setSecret(created.secret);
                setName("");
                notify("Read-only key created. Copy it now.");
              });
            }}
          >
            <label>
              Key name
              <input
                required
                maxLength={120}
                value={name}
                autoComplete="off"
                onChange={(e) => setName(e.target.value)}
              />
            </label>
            <label>
              Application
              <input
                required
                maxLength={120}
                value={application}
                autoComplete="off"
                onChange={(e) => setApplication(e.target.value)}
              />
            </label>
            <label>
              Expiry (optional, this device's local time)
              <input
                type="datetime-local"
                value={expiry}
                onChange={(e) => setExpiry(e.target.value)}
              />
            </label>
            <p className="subtle">
              {expiry && Number.isFinite(new Date(expiry).getTime())
                ? `Expires ${new Date(expiry).toISOString()} (UTC).`
                : "No expiry: valid until you revoke it."}
            </p>
            <button className="primary" disabled={busy}>
              Create read-only key
            </button>
          </form>
          {secret && (
            <section className="api-secret" aria-label="New API key">
              <h3>Copy your key now</h3>
              <p>
                This secret is shown once. It cannot be recovered after leaving
                this page.
              </p>
              <label>
                New key secret
                <input
                  readOnly
                  value={secret}
                  autoComplete="off"
                  spellCheck={false}
                />
              </label>
              <div className="connection-actions">
                <button
                  className="secondary"
                  disabled={busy}
                  onClick={() =>
                    void action(async () => {
                      await navigator.clipboard.writeText(secret);
                      notify("Key copied.");
                    })
                  }
                >
                  Copy key
                </button>
                <button className="secondary" onClick={() => setSecret(null)}>
                  I have saved it
                </button>
              </div>
            </section>
          )}
          <h3>Your keys</h3>
          {!settings.keys.length && <p>No keys yet.</p>}
          {settings.keys.map((key) => (
            <section className="job" key={key.id}>
              <strong>{key.name}</strong>
              <span>
                {key.application} · {key.permissions}
              </span>
              <small>
                Created {date(key.created_at_ms, preferences)} · Last successful
                use {date(key.last_used_at_ms, preferences)}
              </small>
              <span>
                {key.revoked_at_ms
                  ? `Revoked ${date(key.revoked_at_ms, preferences)}`
                  : key.expires_at_ms !== null
                    ? `${key.expires_at_ms <= Date.now() ? "Expired" : "Expires"} ${date(key.expires_at_ms, preferences)}`
                    : "Active · No expiry"}
              </span>
              {!key.revoked_at_ms && (
                <button
                  className="secondary danger"
                  disabled={busy}
                  onClick={() => {
                    if (
                      window.confirm(
                        `Revoke “${key.name}”? New requests will be denied. Data already received by the application is kept.`,
                      )
                    )
                      void action(async () => {
                        await api.revokeApiKey(key.id);
                        notify("Key revoked.");
                      });
                  }}
                >
                  Revoke {key.name}
                </button>
              )}
            </section>
          ))}
          <h3>Allowed browser origins</h3>
          <p>
            Enter one exact HTTPS origin per line, for example
            https://itsmrma.github.io. Do not include a repository path. All
            repositories on that origin share this permission; each request
            still needs a valid key. No origins are allowed initially.
          </p>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void action(async () => {
                await api.saveApiOrigins(
                  origins
                    .split(/\r?\n/)
                    .map((s) => s.trim())
                    .filter(Boolean),
                );
                notify("Allowed origins saved.");
              });
            }}
          >
            <label>
              Allowed origins
              <textarea
                aria-label="Allowed origins"
                rows={3}
                value={origins}
                onChange={(e) => setOrigins(e.target.value)}
                placeholder="https://itsmrma.github.io"
              />
            </label>
            <button className="secondary" disabled={busy}>
              Save allowed origins
            </button>
          </form>
          <p className="subtle">
            If the browser cannot connect, check the URL, LAN/VPN, certificate,
            allowed origin and local-network permission. A generic network error
            does not identify which check failed. Revocation blocks new requests
            and does not erase data already downloaded.
          </p>
        </>
      )}
    </article>
  );
}

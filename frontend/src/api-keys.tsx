import { useEffect, useState } from "react";
import { api } from "./api";
import type { ApiKey, ApiKeySettings, Preferences } from "./generated";
import { CopyButton, date, ErrorText, Icon, Modal, useToast } from "./ui";

export function ApiKeys({ preferences }: { preferences: Preferences }) {
  const [settings, setSettings] = useState<ApiKeySettings | null>(null);
  const [name, setName] = useState("");
  const [application, setApplication] = useState("spotify_analysis");
  const [expiry, setExpiry] = useState("");
  const [origins, setOrigins] = useState("");
  const [secret, setSecret] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [revoking, setRevoking] = useState<ApiKey | null>(null);
  const [revokeError, setRevokeError] = useState("");
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

  async function handleRevoke(target: ApiKey) {
    setBusy(true);
    setRevokeError("");
    setError("");
    try {
      await api.revokeApiKey(target.id);
      setSettings(await api.apiKeys());
      setRevoking(null);
      notify("Key revoked.");
    } catch (e) {
      const message = e instanceof Error ? e.message : "Revocation failed.";
      setRevokeError(message);
      setError(message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <article className="panel padded api-keys-panel">
      <div className="api-keys-header">
        <h2>API keys</h2>
        <p>
          Read-only access to your listening history. Keys cannot change
          settings, import data, access backups or manage Spotify.
        </p>
      </div>

      <ErrorText>{error}</ErrorText>

      {settings && (
        <>
          <section
            className="api-connect-overview"
            aria-labelledby="connect-heading"
          >
            <div className="api-section-header">
              <h3 id="connect-heading">Connect an application</h3>
              <p>
                Connecting an external application (like{" "}
                <strong>spotify_analysis</strong>) requires three coordinated
                settings: the server URL, an allowed browser origin for web
                apps, and a read-only API key.
              </p>
            </div>

            <div className="api-connection-steps">
              <div className="api-step-card">
                <div className="api-step-header">
                  <span className="step-number">1</span>
                  <div>
                    <h4>API base URL</h4>
                    <p className="subtle">
                      The server address your application connects to.
                    </p>
                  </div>
                </div>
                <div className="api-step-content">
                  <label>
                    API base URL
                    <input readOnly value={settings.base_url} />
                  </label>
                  <CopyButton value={settings.base_url} disabled={busy}>
                    Copy API URL
                  </CopyButton>
                  {!settings.base_url.startsWith("https://") ? (
                    <p className="warning">
                      This instance uses HTTP. Configure the external HTTPS
                      origin before connecting from GitHub Pages. HTTP loopback
                      is for local development.
                    </p>
                  ) : (
                    <p className="subtle">
                      Use your private HTTPS address on LAN or VPN. Applications
                      on remote devices cannot reach loopback (127.0.0.1)
                      addresses.
                    </p>
                  )}
                </div>
              </div>

              <div className="api-step-card">
                <div className="api-step-header">
                  <span className="step-number">2</span>
                  <div>
                    <h4>Allowed browser origins</h4>
                    <p className="subtle">
                      Permit web applications to make cross-origin requests
                      (CORS).
                    </p>
                  </div>
                </div>
                <div className="api-step-content">
                  <p>
                    Enter one exact HTTPS origin per line, for example{" "}
                    <code>https://itsmrma.github.io</code> without a repository
                    path. Browser apps are blocked by CORS unless their origin
                    is listed here.
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
                    <p className="subtle">
                      {settings.cors_origins.length
                        ? `Saved: ${settings.cors_origins.join(", ")}`
                        : "No origins saved yet. Enter and save an origin to allow browser requests."}
                    </p>
                    <button className="secondary" disabled={busy}>
                      Save allowed origins
                    </button>
                  </form>
                </div>
              </div>

              <div className="api-step-card">
                <div className="api-step-header">
                  <span className="step-number">3</span>
                  <div>
                    <h4>Create read-only key</h4>
                    <p className="subtle">
                      Generate a secret token for your external application.
                    </p>
                  </div>
                </div>
                <div className="api-step-content">
                  <form
                    className="api-key-create"
                    onSubmit={(e) => {
                      e.preventDefault();
                      void action(async () => {
                        const created = await api.createApiKey({
                          name,
                          application,
                          expires_at_ms: expiry
                            ? new Date(expiry).getTime()
                            : null,
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
                        placeholder="e.g. My laptop browser"
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
                        placeholder="e.g. spotify_analysis"
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
                        This secret is shown once. It cannot be recovered after
                        leaving this page.
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
                        <CopyButton value={secret} disabled={busy}>
                          Copy key
                        </CopyButton>
                        <button
                          className="secondary"
                          onClick={() => setSecret(null)}
                        >
                          I have saved it
                        </button>
                      </div>
                    </section>
                  )}
                </div>
              </div>
            </div>
          </section>

          <section
            className="api-keys-section"
            aria-labelledby="your-keys-heading"
          >
            <div className="api-section-header">
              <h3 id="your-keys-heading">Your keys</h3>
              <p className="subtle">
                Manage active and revoked keys. Revoking a key blocks future
                read access immediately.
              </p>
            </div>

            {!settings.keys.length ? (
              <p className="subtle">No keys yet.</p>
            ) : (
              <div className="api-keys-list">
                {settings.keys.map((key) => {
                  const isRevoked = !!key.revoked_at_ms;
                  const isExpired =
                    !isRevoked &&
                    key.expires_at_ms !== null &&
                    key.expires_at_ms <= Date.now();

                  return (
                    <article
                      className={`api-key-card ${isRevoked ? "revoked" : isExpired ? "expired" : "active"}`}
                      key={key.id}
                    >
                      <div className="api-key-card-header">
                        <strong className="api-key-name">{key.name}</strong>
                        <span
                          className={`state-badge ${isRevoked ? "failed" : isExpired ? "waiting" : "active"}`}
                        >
                          <Icon>
                            {isRevoked
                              ? "cancel"
                              : isExpired
                                ? "schedule"
                                : "check_circle"}
                          </Icon>
                          <span>
                            {isRevoked
                              ? `Revoked ${date(key.revoked_at_ms, preferences)}`
                              : isExpired
                                ? `Expired ${date(key.expires_at_ms, preferences)}`
                                : key.expires_at_ms !== null
                                  ? `Expires ${date(key.expires_at_ms, preferences)}`
                                  : "Active · No expiry"}
                          </span>
                        </span>
                      </div>

                      <div className="api-key-meta">
                        <span className="api-key-meta-item">
                          <Icon>apps</Icon>
                          <span>{key.application}</span>
                        </span>
                        <span className="api-key-meta-item">
                          <Icon>lock</Icon>
                          <span>{key.permissions}</span>
                        </span>
                      </div>

                      <div className="api-key-footer">
                        <div className="api-key-timestamps">
                          <span>
                            Created {date(key.created_at_ms, preferences)}
                          </span>
                          <span className="api-key-sep" aria-hidden="true">
                            ·
                          </span>
                          <span>
                            Last successful use{" "}
                            {date(key.last_used_at_ms, preferences)}
                          </span>
                        </div>
                        {!isRevoked && (
                          <button
                            type="button"
                            className="secondary danger"
                            disabled={busy}
                            onClick={() => {
                              setRevoking(key);
                              setRevokeError("");
                            }}
                          >
                            Revoke {key.name}
                          </button>
                        )}
                      </div>
                    </article>
                  );
                })}
              </div>
            )}
          </section>

          {revoking && (
            <Modal
              title={`Revoke “${revoking.name}”?`}
              close={() => {
                if (!busy) {
                  setRevoking(null);
                  setRevokeError("");
                }
              }}
            >
              <ErrorText>{revokeError}</ErrorText>
              <p>
                Revoke “{revoking.name}”? New requests will be denied. Data
                already received by <strong>{revoking.application}</strong> is
                kept.
              </p>
              <div className="connection-actions">
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  onClick={() => {
                    setRevoking(null);
                    setRevokeError("");
                  }}
                >
                  Cancel
                </button>
                <button
                  type="button"
                  className="secondary danger"
                  disabled={busy}
                  autoFocus
                  onClick={() => void handleRevoke(revoking)}
                >
                  {busy ? "Revoking…" : "Confirm revocation"}
                </button>
              </div>
            </Modal>
          )}

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

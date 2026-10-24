import { AccountSettings, ApplicationSettings } from "./account-settings";
import { Select } from "./select";
import { SpotifyNotice } from "./spotify-status";
import { useEffect, useState } from "react";
import type { CalendarScope, Preferences, SpotifyStatus } from "./generated";
import { api } from "./api";
import {
  CopyButton,
  date,
  ErrorText,
  Heading,
  Icon,
  Modal,
  useToast,
} from "./ui";
import { SpotifyConnectButton } from "./spotify-connect";
import { defaultAccent, ink, isHex, periods } from "./preferences";
import type { useAppearance } from "./preferences";
import { Link, navigate } from "./navigation";
import { Recovery } from "./recovery";
import { ApiKeys } from "./api-keys";

export function General({
  preferences,
  saved,
}: {
  preferences: Preferences;
  saved: (p: Preferences) => void;
}) {
  const [draft, setDraft] = useState(preferences),
    [baseline, setBaseline] = useState(preferences),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [confirm, setConfirm] = useState(false),
    [scope, setScope] = useState<CalendarScope | null>(null);
  const dirty = JSON.stringify(draft) !== JSON.stringify(baseline);
  useEffect(() => {
    if (!dirty) {
      setDraft(preferences);
      setBaseline(preferences);
    }
  }, [preferences]);
  const toast = useToast();
  const update = <K extends keyof Preferences>(key: K, value: Preferences[K]) =>
    setDraft((p) => ({ ...p, [key]: value }));
  async function save(calendar_scope: CalendarScope | null) {
    setBusy(true);
    setError("");
    try {
      const result = await api.savePreferences({
        preferences: draft,
        calendar_scope,
      });
      setDraft(result.preferences);
      setBaseline(result.preferences);
      saved(result.preferences);
      setConfirm(false);
      toast(
        result.job_id
          ? "Preferences saved. The calendar change is queued; follow its progress in Activities."
          : "Preferences saved.",
      );
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      toast(message, true);
    } finally {
      setBusy(false);
    }
  }
  return (
    <article className="panel padded settings-form">
      <h2>General</h2>
      <p>
        These preferences belong to this instance and are shared across
        browsers.
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (
            draft.timezone !== baseline.timezone ||
            draft.week_start !== baseline.week_start
          ) {
            setScope(null);
            setConfirm(true);
          } else void save(null);
        }}
      >
        <label>
          Instance name
          <input
            value={draft.name}
            maxLength={80}
            required
            onChange={(e) => update("name", e.target.value)}
          />
        </label>
        <div className="form-grid">
          <label>
            Initial dashboard period
            <Select
              aria-label="Initial dashboard period"
              value={draft.initial_period}
              onChange={(value) => update("initial_period", value)}
            >
              {periods.map(([value, text]) => (
                <option key={value} value={value}>
                  {text}
                </option>
              ))}
            </Select>
          </label>
          <label>
            Time format
            <Select
              aria-label="Time format"
              value={draft.hour_format}
              onChange={(value) => update("hour_format", value)}
            >
              <option value="24">24-hour</option>
              <option value="12">12-hour</option>
            </Select>
          </label>
          <label>
            Time zone
            <Select
              searchable
              aria-label="Time zone"
              value={draft.timezone}
              onChange={(value) => update("timezone", value)}
            >
              <option value="UTC">UTC</option>
              {Intl.supportedValuesOf("timeZone").map((zone) => (
                <option key={zone} value={zone}>
                  {zone}
                </option>
              ))}
            </Select>
          </label>
          <label>
            Week starts on
            <Select
              aria-label="Week starts on"
              value={draft.week_start}
              onChange={(value) => update("week_start", Number(value))}
            >
              {[
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
              ].map((day, i) => (
                <option key={day} value={i}>
                  {day}
                </option>
              ))}
            </Select>
          </label>
        </div>
        <p>
          Changing the calendar requires a choice before it takes effect.
          Changing the clock format only changes how times are displayed.
        </p>
        <ErrorText>{error}</ErrorText>
        {preferences.version !== baseline.version && dirty && (
          <p className="warning">
            Settings changed while you were editing. Your input is kept.{" "}
            <button
              type="button"
              className="secondary"
              onClick={() => {
                setDraft(preferences);
                setBaseline(preferences);
              }}
            >
              Reload settings
            </button>
          </p>
        )}
        <button className="primary" disabled={busy || !dirty}>
          {busy ? "Saving…" : "Save"}
        </button>
      </form>
      {confirm && (
        <Modal
          title="Apply calendar change"
          close={() => {
            if (!busy) setConfirm(false);
          }}
        >
          <p>
            Choose how {draft.timezone} and the new week start apply. Original
            timestamps and source observations stay intact.
          </p>
          <fieldset>
            <legend>Calendar scope</legend>
            <label className="radio">
              <input
                type="radio"
                name="scope"
                checked={scope === "future"}
                onChange={() => setScope("future")}
              />
              Only listens collected or imported from now on
            </label>
            <p>
              Existing listens keep their assigned calendar. An old January
              listen imported for the first time after Rome → UTC uses UTC; a
              January listen already in the archive keeps Rome. Reimporting that
              existing listen does not change its calendar.
            </p>
            <label className="radio">
              <input
                type="radio"
                name="scope"
                checked={scope === "all"}
                onChange={() => setScope("all")}
              />
              The whole listening history
            </label>
            <p>
              A persistent activity recalculates the entire archive. The new
              calendar becomes active together when the recalculation completes.
              The listen count stays the same.
            </p>
          </fieldset>
          <ErrorText>{error}</ErrorText>
          <div className="connection-actions">
            <button
              className="secondary"
              disabled={busy}
              onClick={() => setConfirm(false)}
            >
              Cancel
            </button>
            <button
              className="primary"
              disabled={busy || !scope}
              onClick={() => void save(scope)}
            >
              {busy ? "Saving…" : "Apply calendar"}
            </button>
          </div>
        </Modal>
      )}
    </article>
  );
}
export function Appearance({
  appearance,
}: {
  appearance: ReturnType<typeof useAppearance>;
}) {
  const { theme, setTheme, accent, setAccent } = appearance;
  const [hex, setHex] = useState(accent);
  useEffect(() => setHex(accent), [accent]);
  const palette = [
    defaultAccent,
    "#91c9f7",
    "#c5a3ee",
    "#f7b68b",
    "#f099ad",
    "#64d9c3",
  ];
  return (
    <article className="panel padded settings-form">
      <h2>Appearance</h2>
      <p>
        Saved immediately in this browser. Other browsers keep their own
        appearance.
      </p>
      <label>
        Theme
        <Select
          aria-label="Theme"
          id="theme"
          value={theme}
          onChange={(value) => setTheme(value)}
        >
          <option value="auto">System</option>
          <option value="light">Light</option>
          <option value="dark">Dark</option>
        </Select>
      </label>
      <fieldset>
        <legend>Accent colour</legend>
        <div className="palette">
          {palette.map((color) => (
            <button
              key={color}
              className="swatch"
              style={{ background: color, color: ink(color) }}
              aria-label={`Use ${color}`}
              aria-pressed={accent === color}
              onClick={() => setAccent(color)}
            >
              {accent === color && <Icon>check</Icon>}
            </button>
          ))}
        </div>
        <div className="connection-actions">
          <label>
            Pick a colour
            <input
              type="color"
              value={accent}
              onChange={(e) => setAccent(e.target.value)}
            />
          </label>
          <label>
            HEX colour
            <input
              value={hex}
              maxLength={7}
              aria-invalid={!isHex(hex)}
              onChange={(e) => {
                setHex(e.target.value);
                if (isHex(e.target.value)) setAccent(e.target.value);
              }}
            />
          </label>
        </div>
        {!isHex(hex) && (
          <p className="form-error">
            Enter a six-digit HEX colour, for example #c9ee8b.
          </p>
        )}
        <button className="secondary" onClick={() => setAccent(defaultAccent)}>
          Reset accent
        </button>
      </fieldset>
      <div className="previews">
        {["light", "dark"].map((mode) => (
          <div className={`preview ${mode}`} key={mode}>
            <strong>{mode === "light" ? "Light" : "Dark"} preview</strong>
            <p>Your listening archive</p>
            <span style={{ background: accent, color: ink(accent) }}>
              Selected action
            </span>
            <p className="preview-error">An error remains recognisable.</p>
          </div>
        ))}
      </div>
    </article>
  );
}
function HttpsGuide() {
  return (
    <details className="guide">
      <summary>
        <Icon>lock</Icon>
        <span>
          <strong>Connect on a private server with HTTPS</strong>
          <small>Private address, certificate and reverse proxy</small>
        </span>
        <Icon>expand_more</Icon>
      </summary>
      <p>
        Use a trusted HTTPS name reachable over LAN or VPN. Keep scrobbl.rs
        private; this setup does not require router port forwarding. Local use
        on 127.0.0.1 can continue with its loopback callback.
      </p>
      <details>
        <summary>I don’t have a domain — use DuckDNS</summary>
        <ol>
          <li>
            Register a free name at{" "}
            <a href="https://www.duckdns.org/" target="_blank" rel="noreferrer">
              DuckDNS
            </a>
            .
          </li>
          <li>
            In your certificate manager, use the DuckDNS DNS integration and
            token for a Let’s Encrypt DNS-01 challenge. Keep the token in that
            manager, with persistent certificate/account data and automatic
            renewal.
          </li>
          <li>
            Resolve that exact hostname to your reverse proxy’s private LAN
            address. In AdGuard Home, add a DNS rewrite for the exact name. VPN
            clients must use DNS that resolves and reaches that address.
          </li>
        </ol>
      </details>
      <details>
        <summary>I already have a domain</summary>
        <ol>
          <li>
            Choose a subdomain you control and resolve it to your proxy’s
            private address on LAN/VPN.
          </li>
          <li>
            Use a trusted certificate you already manage, or DNS-01 through your
            authoritative DNS provider. Its integration and credentials may
            differ from your registrar; automation depends on the provider.
          </li>
          <li>
            Enable automatic renewal and persist the certificate manager’s data.
            Keep DNS credentials there.
          </li>
        </ol>
      </details>
      <h3>Reverse proxy: the data you need</h3>
      <ol>
        <li>
          Choose the external HTTPS origin, including any nonstandard port. The
          proxy must listen on that private reachable port and serve a trusted
          certificate for the name.
        </li>
        <li>
          Set its upstream to scrobbl.rs’s internal HTTP address and port.
          Ensure the proxy can reach the app across their private/container
          network.
        </li>
        <li>
          Forward the full path and query and preserve the external Host header,
          including any nonstandard port. scrobbl.rs ignores Forwarded and
          X-Forwarded-* headers. Restrict its upstream port to the proxy’s
          private network; there is no trusted forwarding-header list to
          configure.
        </li>
        <li>
          Set SCROBBL_EXTERNAL_ORIGIN to that exact HTTPS origin and restart the
          app. SCROBBL_BIND and SCROBBL_PORT control the internal HTTP listener
          (0.0.0.0:3210 inside Docker). Open the HTTPS name in your browser and
          sign in there before connecting Spotify.
        </li>
        <li>
          Copy the exact redirect URI displayed below into Spotify. Match
          scheme, hostname, port and callback path. Do not change the host in
          the middle of an authorization attempt.
        </li>
        <li>
          Verify private DNS, certificate trust/chain, callback return to the
          same authenticated session and automatic certificate renewal. After an
          interrupted authorization, start Connect Spotify again.
        </li>
      </ol>
      <p>
        If the browser reports a certificate error, fix DNS, the hostname and
        full certificate chain before signing in. A 403 from the app usually
        means the proxy changed Host or the browser uses a different origin. A
        missing OAuth session means you must sign in at the configured HTTPS
        name and start again. For failed renewals, check the certificate
        manager’s DNS permissions, logs and persistent storage. Never bypass a
        certificate error. DNS tokens belong only in the certificate manager.
      </p>
    </details>
  );
}
function RefreshIntervals({
  status,
  refresh,
}: {
  status: SpotifyStatus;
  refresh: () => Promise<void>;
}) {
  const incoming = {
    sync: String(status.sync_seconds),
    now: String(status.now_playing_seconds),
  };
  const [draft, setDraft] = useState(incoming);
  const [baseline, setBaseline] = useState(incoming);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const toast = useToast();
  const dirty = draft.sync !== baseline.sync || draft.now !== baseline.now;
  useEffect(() => {
    if (!dirty) {
      setDraft(incoming);
      setBaseline(incoming);
    }
  }, [status.sync_seconds, status.now_playing_seconds]);
  return (
    <form
      className="settings-form"
      onSubmit={async (event) => {
        event.preventDefault();
        setBusy(true);
        setError("");
        try {
          const result = await api.saveSpotifyIntervals({
            sync_seconds: Number(draft.sync),
            now_playing_seconds: Number(draft.now),
          });
          const saved = {
            sync: String(result.sync_seconds),
            now: String(result.now_playing_seconds),
          };
          setDraft(saved);
          setBaseline(saved);
          await refresh();
          toast(
            "Spotify intervals saved. Changes apply without restarting the server.",
          );
        } catch (error) {
          setError(error instanceof Error ? error.message : String(error));
        } finally {
          setBusy(false);
        }
      }}
    >
      <h3>Refresh intervals</h3>
      <p>
        Saved for this instance and shared across browsers. Changes apply while
        the server is running.
      </p>
      <div className="form-grid">
        <label>
          Collection interval (seconds)
          <input
            type="number"
            min="60"
            max="86400"
            step="1"
            required
            value={draft.sync}
            disabled={busy}
            onChange={(event) =>
              setDraft((value) => ({ ...value, sync: event.target.value }))
            }
          />
        </label>
        <label>
          Now playing interval (seconds)
          <input
            type="number"
            min="15"
            max="86400"
            step="1"
            required
            value={draft.now}
            disabled={busy}
            onChange={(event) =>
              setDraft((value) => ({ ...value, now: event.target.value }))
            }
          />
        </label>
      </div>
      <p className="subtle">
        Collection: at least 60 seconds. Now playing: at least 15 seconds, while
        the page is visible. Spotify availability and request limits can add
        delay.
      </p>
      <ErrorText>{error}</ErrorText>
      <button className="secondary" disabled={busy || !dirty}>
        Save intervals
      </button>
    </form>
  );
}

export function Connections({
  status,
  preferences,
  refresh,
  statusError,
}: {
  status: SpotifyStatus | null;
  preferences: Preferences;
  refresh: () => Promise<void>;
  statusError: string;
}) {
  const [open, setOpen] = useState(false),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [client, setClient] = useState(""),
    [secret, setSecret] = useState("");
  const toast = useToast();
  const waiting = !!status?.retry_at_ms && status.retry_at_ms > Date.now();
  const needsAction =
    status?.connection === "action_required" ||
    (!!status?.last_error && !waiting);
  useEffect(() => {
    if (needsAction || (status && !status.configured)) setOpen(true);
  }, [needsAction, status?.configured, status?.last_error]);
  async function action(run: () => Promise<string | void>) {
    setBusy(true);
    setError("");
    try {
      const message = await run();
      await refresh();
      if (message) toast(message);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setError(message);
      toast(message, true);
      await refresh().catch(() => {});
    } finally {
      setBusy(false);
    }
  }
  if (!status)
    return (
      <>
        <ErrorText>{statusError}</ErrorText>
        <p>Loading connection status…</p>
      </>
    );
  const connection = !status.connected
    ? status.configured
      ? "Ready to connect"
      : "App setup needed"
    : status.connection === "action_required"
      ? "Action required — reconnect or check permissions"
      : status.connection === "verified"
        ? "Authorization verified on last request"
        : "Authorization saved — not yet verified";
  const collection =
    {
      paused: "Paused",
      waiting: "Waiting for Spotify’s request limit",
      error: "Collection error — will retry",
      active: status.last_sync_ms
        ? "Active — background collection scheduled"
        : "Active — awaiting first collection",
    }[status.collection] || status.collection;
  return (
    <article className="panel padded">
      <h2>Connections</h2>
      <p>
        Collection runs while scrobbl.rs is running, even with the browser
        closed.
      </p>
      <ErrorText>{statusError}</ErrorText>
      <details
        className="spotify-card"
        open={open}
        onToggle={(e) => setOpen(e.currentTarget.open)}
      >
        <summary>
          <div className="connection-identity">
            <span className="connection-provider-icon">
              <Icon>headphones</Icon>
            </span>
            <div>
              <strong className="connection-title">Spotify</strong>
              <span className="connection-collection">{collection}</span>
            </div>
          </div>
          <span
            id="connection-badge"
            className={`connection-badge ${status.connected && status.connection === "verified" ? "connection-verified" : ""}`}
          >
            <Icon>
              {status.connected && status.connection === "verified"
                ? "check_circle"
                : status.connection === "action_required"
                  ? "error_outline"
                  : "link"}
            </Icon>
            <span>
              {status.connected && status.connection === "verified" ? (
                <>
                  <strong>Connected</strong>
                  <small>Verified on last request</small>
                </>
              ) : (
                connection
              )}
            </span>
          </span>
          <Icon>expand_more</Icon>
          {needsAction && (
            <span>Review the details and the available actions.</span>
          )}
        </summary>
        <div className="spotify-details">
          <SpotifyNotice status={status} preferences={preferences} />
          <ol
            className="connection-steps"
            aria-label="Spotify connection steps"
          >
            <li className="connection-step">
              <div className="step-heading">
                <span className="step-number">1</span>
                <div>
                  <h3>Prepare your private address</h3>
                  <p>
                    On a server, use trusted HTTPS over LAN or VPN. Local
                    127.0.0.1 works directly.
                  </p>
                </div>
              </div>
              <HttpsGuide />
            </li>
            <li className="connection-step">
              <div className="step-heading">
                <span className="step-number">2</span>
                <div>
                  <h3>Set up your Spotify app</h3>
                  {status.configured && (
                    <span className="credentials-saved" role="status">
                      <Icon>check_circle</Icon>App credentials saved
                    </span>
                  )}
                  <p>
                    {status.configured
                      ? "Open the setup guide to review or replace your saved app credentials."
                      : "Register the redirect URI, save it in Spotify, then save your app credentials here."}
                  </p>
                </div>
              </div>
              <details className="guide" open={!status.configured}>
                <summary>
                  <Icon>settings</Icon>
                  <span>
                    <strong>Create or configure your Spotify app</strong>
                    <small>Redirect URI and app credentials</small>
                  </span>
                  <Icon>expand_more</Icon>
                </summary>
                <p>
                  Complete these steps in order. Keep the Developer Dashboard
                  open alongside scrobbl.rs.
                </p>
                <ol>
                  <li>
                    Open the{" "}
                    <a
                      href="https://developer.spotify.com/dashboard"
                      target="_blank"
                      rel="noreferrer"
                    >
                      Spotify Developer Dashboard
                    </a>{" "}
                    and create an app for your listening archive, selecting Web
                    API.
                  </li>
                  <li>
                    Register this exact redirect URI:
                    <code id="redirect-uri">{status.redirect_uri}</code>
                    <CopyButton value={status.redirect_uri} disabled={busy}>
                      Copy redirect URI
                    </CopyButton>
                  </li>
                  <li>
                    Click <strong>Save</strong> in the Spotify Developer
                    Dashboard after adding the redirect URI. Unsaved changes
                    cause “redirect_uri: Not matching configuration”. Check app
                    eligibility and permitted users, then copy the Client ID and
                    Client Secret into the fields below.
                  </li>
                </ol>
                {status.credentials_from_env ? (
                  <p>
                    Spotify app credentials are managed by the server
                    environment. Change both values there and restart the
                    server.
                  </p>
                ) : (
                  <form
                    onSubmit={(e) => {
                      e.preventDefault();
                      void action(async () => {
                        await api.credentials(client, secret);
                        setClient("");
                        setSecret("");
                        return "Spotify app saved. Choose Connect Spotify to authorize your account.";
                      });
                    }}
                  >
                    <label>
                      Client ID
                      <input
                        value={client}
                        required
                        maxLength={256}
                        autoComplete="off"
                        onChange={(e) => setClient(e.target.value)}
                      />
                    </label>
                    <label>
                      Client Secret
                      <input
                        type="password"
                        value={secret}
                        required
                        maxLength={256}
                        autoComplete="off"
                        onChange={(e) => setSecret(e.target.value)}
                      />
                    </label>
                    <p className="subtle">
                      Stored on the server. Saved credentials are never returned
                      to the browser.
                    </p>
                    <button className="secondary" disabled={busy}>
                      Save Spotify app
                    </button>
                  </form>
                )}
              </details>
            </li>
            <li className="connection-step">
              <div className="step-heading">
                <span className="step-number">3</span>
                <div>
                  <h3>Authorize your account</h3>
                  <p>
                    {status.connected
                      ? "Authorization is saved. Review collection status below, or reconnect when needed."
                      : status.configured
                        ? "Ready to connect. Spotify opens in a separate tab; this page shows the result."
                        : "Save your app credentials in step 2 to enable Connect Spotify."}
                  </p>
                </div>
              </div>
              <ErrorText>{error}</ErrorText>
              <div className="connection-actions">
                <SpotifyConnectButton
                  connected={status.connected}
                  disabled={busy || !status.configured}
                  refresh={refresh}
                />
                <button
                  className="secondary"
                  disabled={
                    busy ||
                    !status.connected ||
                    (!!status.retry_at_ms && status.retry_at_ms > Date.now())
                  }
                  onClick={() =>
                    void action(async () => {
                      const result = await api.sync();
                      return `Connection checked and sync complete: ${result.added} new listens, ${result.reconciled} reconciled, ${result.duplicates} already present.`;
                    })
                  }
                >
                  Verify & sync now
                </button>
                <button
                  className="secondary danger"
                  disabled={busy || !status.connected}
                  onClick={() =>
                    void action(async () => {
                      await api.disconnect();
                      return "Spotify disconnected. Your listening archive is kept.";
                    })
                  }
                >
                  Disconnect
                </button>
              </div>
            </li>
          </ol>
          <details className="connection-monitor" open={status.connected}>
            <summary>Collection status &amp; diagnostics</summary>
            <p>
              Authorization and collection are separate. A saved authorization
              is not a continuous health check.
            </p>
            <dl className="diagnostics">
              <dt>Authorization last verified</dt>
              <dd>{date(status.verified_at_ms, preferences)}</dd>
              <dt>Last collection attempt</dt>
              <dd>{date(status.last_attempt_ms, preferences)}</dd>
              <dt>Last successful collection</dt>
              <dd>{date(status.last_sync_ms, preferences)}</dd>
              <dt>New listens in last successful collection</dt>
              <dd>
                {status.last_added === null
                  ? "Not yet known"
                  : status.last_added === 0
                    ? "0 — successful, no new listens"
                    : status.last_added}
              </dd>
              <dt>Collection interval</dt>
              <dd>Every {status.sync_seconds} seconds</dd>
              <dt>Next attempt / request limit</dt>
              <dd>
                {status.retry_at_ms && status.retry_at_ms > Date.now()
                  ? date(status.retry_at_ms, preferences)
                  : status.connected && status.last_attempt_ms
                    ? date(
                        Math.max(
                          Date.now(),
                          status.last_attempt_ms + status.sync_seconds * 1000,
                        ),
                        preferences,
                      )
                    : "After authorization"}
              </dd>
            </dl>
            {status.now_playing_timing && (
              <details>
                <summary>Last now-playing sample timing</summary>
                <p>
                  Request:{" "}
                  {date(
                    status.now_playing_timing.source_requested_at_ms,
                    preferences,
                  )}
                  . Response:{" "}
                  {date(
                    status.now_playing_timing.response_received_at_ms,
                    preferences,
                  )}
                  .
                </p>
                <p>
                  Server queue:{" "}
                  {status.now_playing_timing.source_requested_at_ms -
                    status.now_playing_timing.requested_at_ms}{" "}
                  ms. Provider request:{" "}
                  {status.now_playing_timing.response_received_at_ms -
                    status.now_playing_timing.source_requested_at_ms}{" "}
                  ms.
                </p>
                <p>
                  Source playback-state change:{" "}
                  {date(
                    status.now_playing_timing.source_timestamp_ms,
                    preferences,
                  )}
                  . This source timestamp does not measure propagation delay.
                  Now playing never creates listens.
                </p>
              </details>
            )}
            <div className="connection-report-actions">
              <Link className="button secondary" to="/activities">
                <Icon>receipt_long</Icon>Collection and import reports
              </Link>
            </div>
            {status.coverage_warning && (
              <p className="warning">{status.coverage_warning}</p>
            )}
            {status.review_count > 0 && (
              <div className="connection-review-actions">
                <span className="warning">
                  {status.review_count} records need review.
                </span>
                <Link className="button secondary" to="/review">
                  <Icon>rule</Icon>Review possible duplicates
                </Link>
              </div>
            )}
          </details>
          <details className="connection-monitor">
            <summary>Refresh intervals</summary>
            <RefreshIntervals status={status} refresh={refresh} />
          </details>
          <p>
            You can also revoke access in your Spotify account’s Apps settings.
            Disconnecting keeps the listening archive.
          </p>
        </div>
      </details>
    </article>
  );
}
export function DataRecovery({
  status,
  preferences,
}: {
  status: SpotifyStatus | null;
  preferences: Preferences;
}) {
  return (
    <article className="panel padded">
      <h2>Data & recovery</h2>
      <p>Your archive’s operations and saved reports.</p>
      <Link to="/activities" className="button secondary">
        View persistent activities
      </Link>
      <Recovery preferences={preferences} />
      <h3>Recent imports</h3>
      {!status?.imports.length && <p>No imports yet.</p>}
      {status?.imports.map((job, i) => (
        <div className="job" key={i}>
          <strong>{job.filename}</strong>
          <small>{date(job.created_at_ms, preferences)}</small>
          <span>
            {job.report.added} added · {job.report.reconciled} reconciled ·{" "}
            {job.report.duplicates} duplicates · {job.report.invalid} invalid
          </span>
          {job.report.ambiguous > 0 && (
            <p className="warning">
              {job.report.ambiguous} ambiguous matches kept separate
            </p>
          )}
          {job.report.errors.map((error, n) => (
            <p className="form-error" key={n}>
              {error}
            </p>
          ))}
        </div>
      ))}
    </article>
  );
}
export function Settings({
  section,
  preferences,
  saved,
  appearance,
  status,
  refresh,
  statusError,
}: {
  section: string;
  preferences: Preferences;
  saved: (p: Preferences) => void;
  appearance: ReturnType<typeof useAppearance>;
  status: SpotifyStatus | null;
  refresh: () => Promise<void>;
  statusError: string;
}) {
  return (
    <div className="settings-page">
      <Heading eyebrow="MAKE YOURSELF AT HOME" title="Settings & connections">
        Preferences for your instance and this browser.
      </Heading>
      <label className="settings-section-select">
        Settings section
        <Select
          aria-label="Settings section"
          value={section}
          onChange={(value) => navigate(`/settings/${value}`)}
        >
          <option value="general">General</option>
          <option value="account">Account</option>
          <option value="application">Application</option>
          <option value="appearance">Appearance</option>
          <option value="connections">Connections</option>
          <option value="data">Data &amp; recovery</option>
          <option value="api-keys">API keys</option>
        </Select>
      </label>
      <nav className="settings-tabs" aria-label="Settings sections">
        {[
          ["general", "General"],
          ["account", "Account"],
          ["application", "Application"],
          ["appearance", "Appearance"],
          ["connections", "Connections"],
          ["data", "Data & recovery"],
          ["api-keys", "API keys"],
        ].map(([key, title]) => (
          <Link
            key={key}
            to={`/settings/${key}`}
            aria-current={section === key ? "page" : undefined}
          >
            {title}
          </Link>
        ))}
      </nav>
      {section === "account" && <AccountSettings />}
      {section === "application" && <ApplicationSettings />}
      <div hidden={section !== "general"}>
        <General preferences={preferences} saved={saved} />
      </div>
      <div hidden={section !== "appearance"}>
        <Appearance appearance={appearance} />
      </div>
      <div hidden={section !== "connections"}>
        <Connections
          status={status}
          preferences={preferences}
          refresh={refresh}
          statusError={statusError}
        />
      </div>
      <div hidden={section !== "data"}>
        <DataRecovery status={status} preferences={preferences} />
      </div>
      <div hidden={section !== "api-keys"}>
        {section === "api-keys" && <ApiKeys preferences={preferences} />}
      </div>
    </div>
  );
}

import { useState } from "react";
import { api, setCsrf } from "./api";
import { useAppearance } from "./preferences";
import { ErrorText, Icon, Wordmark } from "./ui";

export function passwordChecks(password: string, username = "") {
  const compact = password.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
  const common = [
    "passwordpassword",
    "password123456789",
    "123456789012345",
    "1234567890123456",
    "qwertyuiopasdfgh",
    "qwertyuiopasdfghjkl",
    "letmeinletmeinletmein",
    "scrobblpassword",
    "scrobblrspassword",
  ];
  const accountName = username.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
  const chars = Array.from(password);
  const repeated =
    chars.length > 0 &&
    [1, 2, 3, 4].some(
      (n) =>
        chars.length % n === 0 && chars.every((c, i) => c === chars[i % n]),
    );
  return [
    { label: "At least 15 characters", met: chars.length >= 15 },
    {
      label: "Not a common or repeating password",
      met: password.length > 0 && !common.includes(compact) && !repeated,
    },
    {
      label: "Different from your username",
      met:
        password.length > 0 &&
        (!username ||
          ![
            accountName,
            `${accountName}password`,
            `${accountName}123456789`,
          ].includes(compact)),
    },
    {
      label: "Within the 1024-byte limit",
      met: new TextEncoder().encode(password).length <= 1024,
    },
  ];
}
export function PasswordFeedback({
  password,
  username = "",
}: {
  password: string;
  username?: string;
}) {
  return (
    <div className="password-guidance" id="password-guidance">
      <p>
        Use a unique passphrase or a password manager. Spaces and pasted
        passwords are welcome.
      </p>
      <ul>
        {passwordChecks(password, username).map(({ label, met }) => (
          <li key={label} className={met ? "requirement-met" : ""}>
            <Icon>{met ? "check_circle" : "radio_button_unchecked"}</Icon>
            {label}
          </li>
        ))}
      </ul>
    </div>
  );
}
export function UsernameSetup({ done }: { done: () => Promise<void> }) {
  const [username, setUsername] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <main className="account-upgrade panel padded">
      <h1>Choose your username</h1>
      <p>
        Your archive now supports a username for signing in. Add yours to finish
        setting up your account.
      </p>
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError("");
          try {
            await api.setUsername(username);
            await done();
          } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        <Username value={username} change={setUsername} />
        <ErrorText>{error}</ErrorText>
        <button className="primary" disabled={busy}>
          Save username and open archive<Icon>arrow_forward</Icon>
        </button>
      </form>
    </main>
  );
}
function Username({
  value,
  change,
}: {
  value: string;
  change: (s: string) => void;
}) {
  return (
    <label>
      Username
      <input
        aria-label="Username"
        value={value}
        onChange={(e) => change(e.target.value)}
        autoComplete="username"
        autoCapitalize="none"
        spellCheck={false}
        required
        minLength={3}
        maxLength={40}
        pattern="[a-zA-Z0-9._\-]+"
        autoFocus
        aria-describedby="username-hint"
      />
      <small id="username-hint">
        3–40 letters, numbers, dots, underscores or hyphens. Not case sensitive.
      </small>
    </label>
  );
}
export function Auth({
  setup,
  usernameRequired,
  done,
}: {
  setup: boolean;
  usernameRequired: boolean;
  done: () => Promise<void>;
}) {
  useAppearance();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [show, setShow] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const valid = passwordChecks(password, username).every((c) => c.met);
  return (
    <div className="auth-layout">
      <div className="auth-story">
        <a className="brand" href="/">
          <img src="/favicon.svg" alt="" width="34" height="34" />
          <Wordmark />
        </a>
        <p className="eyebrow">YOUR MUSIC, REMEMBERED.</p>
        <h1>
          A little archive.
          <br />A lifetime of listening.
        </h1>
        <p>
          Your favourite songs, the late-night repeats, the artists you keep
          coming back to. All in one place, on your own server.
        </p>
        <span className="local-label">Private by default · Stored locally</span>
      </div>
      <main className="auth-card">
        <h2>{setup ? "Make this space yours" : "Welcome back"}</h2>
        <p>
          {setup
            ? "Create your owner account to start your listening archive."
            : "Sign in to your listening archive."}
        </p>
        {!setup && !usernameRequired && (
          <p className="subtle">
            Sign in with your existing password. You’ll choose a username next.
          </p>
        )}
        <form
          onSubmit={async (e) => {
            e.preventDefault();
            setBusy(true);
            setError("");
            try {
              if (setup && !valid)
                throw new Error("Check the password requirements below.");
              if (setup && password !== confirmation)
                throw new Error("The passwords do not match.");
              const result = await api.login(password, setup, username);
              setCsrf(result.csrf);
              setPassword("");
              setConfirmation("");
              await done();
            } catch (e) {
              setError(e instanceof Error ? e.message : String(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          {(setup || usernameRequired) && (
            <Username value={username} change={setUsername} />
          )}
          <label>
            Password
            <div className="password-field">
              <input
                aria-label="Password"
                type={show ? "text" : "password"}
                value={password}
                required
                maxLength={1024}
                autoComplete={setup ? "new-password" : "current-password"}
                onChange={(e) => setPassword(e.target.value)}
                aria-describedby={setup ? "password-guidance" : undefined}
              />
              <button
                type="button"
                className="icon-button"
                aria-label={show ? "Hide password" : "Show password"}
                aria-pressed={show}
                onClick={() => setShow(!show)}
              >
                <Icon>{show ? "visibility_off" : "visibility"}</Icon>
              </button>
            </div>
          </label>
          {setup && (
            <>
              <PasswordFeedback password={password} username={username} />
              <label>
                Confirm password
                <input
                  type={show ? "text" : "password"}
                  value={confirmation}
                  required
                  maxLength={1024}
                  autoComplete="new-password"
                  onChange={(e) => setConfirmation(e.target.value)}
                />
              </label>
              {confirmation && (
                <p
                  className={
                    confirmation === password ? "requirement-met" : "subtle"
                  }
                >
                  {confirmation === password
                    ? "Passwords match"
                    : "Passwords do not match yet"}
                </p>
              )}
            </>
          )}
          <ErrorText>{error}</ErrorText>
          <button
            className="primary"
            aria-busy={busy}
            disabled={busy || (setup && (!valid || password !== confirmation))}
          >
            {setup ? "Create your archive" : "Sign in"}
            <Icon>arrow_forward</Icon>
          </button>
        </form>
        <p className="subtle">
          A personal space for your music.
          <br />
          No shared account. No cloud database.
        </p>
      </main>
    </div>
  );
}

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { api, setCsrf } from "./api";
import type { AccountProfile } from "./api";
import { useLoad } from "./load";
import { SkeletonCard } from "./skeleton";
import { Link } from "./navigation";
import { ErrorText, Icon, Modal, useToast } from "./ui";
import { PasswordFeedback, passwordChecks } from "./account";

function useAccount() {
  return useLoad(api.account, "account", true, undefined, true);
}
async function prepareAvatar(file: File): Promise<Blob> {
  if (!["image/png", "image/jpeg", "image/webp"].includes(file.type)) {
    throw new Error("Choose a PNG, JPEG or WebP photo.");
  }
  const image = await createImageBitmap(file);
  try {
    const scale = Math.min(1, 512 / Math.max(image.width, image.height));
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(image.width * scale));
    canvas.height = Math.max(1, Math.round(image.height * scale));
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Could not prepare this photo.");
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    return await new Promise<Blob>((resolve, reject) =>
      canvas.toBlob(
        (blob) =>
          blob
            ? resolve(blob)
            : reject(new Error("Could not prepare this photo.")),
        "image/png",
      ),
    );
  } finally {
    image.close();
  }
}
function Avatar({
  profile,
  large = false,
}: {
  profile: AccountProfile | null;
  large?: boolean;
}) {
  return (
    <span className={`account-avatar ${large ? "large" : ""}`}>
      {profile?.avatar_url ? (
        <img src={profile.avatar_url} alt="" />
      ) : (
        <Icon>person</Icon>
      )}
    </span>
  );
}

export function AccountMenu({ logout }: { logout: () => void }) {
  const profile = useAccount();
  const menu = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [action, setAction] = useState<"restart" | "stop" | null>(null);
  const toast = useToast();
  const close = () => menu.current?.hidePopover();
  return (
    <>
      <button
        className="icon-button account-trigger"
        aria-label="Account menu"
        popoverTarget="account-menu"
        aria-expanded={open}
        onClick={(event) => event.currentTarget.focus()}
      >
        <Avatar profile={profile.data} />
      </button>
      <div
        popover="auto"
        id="account-menu"
        className="account-menu panel"
        ref={menu}
        onToggle={(e) => setOpen(e.newState === "open")}
      >
        <div className="account-menu-heading">
          <Avatar profile={profile.data} />
          <div>
            <strong>
              {profile.data?.name || profile.data?.username || "Your account"}
            </strong>
            <small>Owner account</small>
          </div>
        </div>
        <ErrorText>{profile.error}</ErrorText>
        <nav aria-label="Account actions">
          <Link to="/settings/account" onClick={close}>
            <Icon>manage_accounts</Icon>Account settings
          </Link>
          <Link to="/settings/application" onClick={close}>
            <Icon>tune</Icon>Application settings
          </Link>
          <hr />
          <button
            onClick={() => {
              close();
              setAction("restart");
            }}
          >
            <Icon>restart_alt</Icon>Restart application
          </button>
          <button
            onClick={() => {
              close();
              setAction("stop");
            }}
          >
            <Icon>power_settings_new</Icon>Stop application
          </button>
          <hr />
          <button
            onClick={async () => {
              try {
                await api.logout();
                close();
                setCsrf(null);
                logout();
              } catch (e) {
                toast(e instanceof Error ? e.message : String(e), true);
              }
            }}
          >
            <Icon>logout</Icon>Sign out
          </button>
        </nav>
      </div>
      {action && (
        <ApplicationDialog action={action} close={() => setAction(null)} />
      )}
    </>
  );
}

export function AccountSettings() {
  const profile = useAccount();
  const [name, setName] = useState("");
  const [current, setCurrent] = useState("");
  const [password, setPassword] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const toast = useToast();
  const initialized = useRef(false);
  useLayoutEffect(() => {
    if (profile.data && !initialized.current) {
      setName(profile.data.name);
      initialized.current = true;
    }
  }, [profile.data]);
  const save = async (operation: () => Promise<unknown>, message: string) => {
    setBusy(true);
    setError("");
    try {
      await operation();
      toast(message);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="account-settings">
      <article className="panel padded settings-form">
        <h2>Your account</h2>
        <p>
          Your profile belongs to this archive. Your photo appears in the
          account menu.
        </p>
        <ErrorText>{profile.error || error}</ErrorText>
        {!profile.data && !profile.error && (
          <SkeletonCard label="Loading account…" />
        )}
        {profile.data && (
          <>
            <div className="profile-photo-row">
              <Avatar profile={profile.data} large />
              <div>
                <label className="profile-upload">
                  Profile photo
                  <input
                    type="file"
                    accept="image/png,image/jpeg,image/webp"
                    disabled={busy}
                    onChange={(e) => {
                      const file = e.target.files?.[0];
                      e.target.value = "";
                      if (!file) return;
                      void save(
                        async () => api.uploadAvatar(await prepareAvatar(file)),
                        "Profile photo saved.",
                      );
                    }}
                  />
                </label>
                <small>
                  PNG, JPEG or WebP · Large photos are resized automatically
                </small>
                {profile.data.avatar_url && (
                  <button
                    className="secondary"
                    disabled={busy}
                    onClick={() =>
                      void save(api.removeAvatar, "Profile photo removed.")
                    }
                  >
                    <Icon>delete</Icon>Remove photo
                  </button>
                )}
              </div>
            </div>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                void save(() => api.saveAccount(name), "Profile saved.");
              }}
            >
              <label>
                Username
                <input
                  value={profile.data.username || ""}
                  readOnly
                  autoComplete="username"
                />
              </label>
              <label>
                Display name
                <input
                  value={name}
                  maxLength={80}
                  onChange={(e) => setName(e.target.value)}
                />
              </label>
              <button className="primary" disabled={busy}>
                <Icon>save</Icon>Save profile
              </button>
            </form>
          </>
        )}
      </article>
      <article className="panel padded settings-form">
        <h2>Password & access</h2>
        <p>
          Changing your password signs out other browser sessions. This browser
          stays signed in.
        </p>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (password !== confirmation) {
              setError("The passwords do not match.");
              return;
            }
            void save(async () => {
              await api.changePassword(current, password);
              setCurrent("");
              setPassword("");
              setConfirmation("");
            }, "Password changed. Other browser sessions have been signed out.");
          }}
        >
          <label>
            Current password
            <input
              type="password"
              autoComplete="current-password"
              required
              value={current}
              maxLength={1024}
              onChange={(e) => setCurrent(e.target.value)}
            />
          </label>
          <label>
            New password
            <input
              type="password"
              autoComplete="new-password"
              required
              value={password}
              maxLength={1024}
              onChange={(e) => setPassword(e.target.value)}
            />
          </label>
          <label>
            Confirm new password
            <input
              type="password"
              autoComplete="new-password"
              required
              value={confirmation}
              maxLength={1024}
              onChange={(e) => setConfirmation(e.target.value)}
            />
          </label>
          <PasswordFeedback
            password={password}
            username={profile.data?.username || ""}
          />
          <button
            className="primary"
            disabled={
              busy ||
              !passwordChecks(password, profile.data?.username || "").every(
                (c) => c.met,
              )
            }
          >
            <Icon>lock</Icon>Change password
          </button>
        </form>
      </article>
    </div>
  );
}

export function ApplicationSettings() {
  const [action, setAction] = useState<"restart" | "stop" | null>(null);
  return (
    <article className="panel padded settings-form">
      <h2>Application</h2>
      <p>Manage scrobbl.rs without restarting or shutting down your host.</p>
      <div className="application-action">
        <div>
          <h3>Restart application</h3>
          <p>
            Close the archive safely and start scrobbl.rs again. Saved data and
            account access are retained.
          </p>
        </div>
        <button className="secondary" onClick={() => setAction("restart")}>
          <Icon>restart_alt</Icon>Restart application
        </button>
      </div>
      <div className="application-action">
        <div>
          <h3>Stop application</h3>
          <p>
            Stop collection and close scrobbl.rs. Start the application again
            from Docker or your process manager.
          </p>
        </div>
        <button className="secondary" onClick={() => setAction("stop")}>
          <Icon>power_settings_new</Icon>Stop application
        </button>
      </div>
      {action && (
        <ApplicationDialog action={action} close={() => setAction(null)} />
      )}
    </article>
  );
}
function ApplicationDialog({
  action,
  close,
}: {
  action: "restart" | "stop";
  close: () => void;
}) {
  const [phase, setPhase] = useState("confirm");
  const [error, setError] = useState("");
  const boot = useRef("");
  useEffect(() => {
    if (phase !== "restarting") return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const until = Date.now() + 60000;
    const poll = async () => {
      try {
        const response = await fetch("/api/v1/application", {
          signal: controller.signal,
        });
        if (response.ok && (await response.json()).boot_id !== boot.current) {
          if (!controller.signal.aborted) {
            setPhase("restarted");
            window.dispatchEvent(new Event("read-data-changed"));
          }
          return;
        }
      } catch {
        /* The listener is unavailable while the archive reopens. */
      }
      if (controller.signal.aborted) return;
      if (Date.now() > until) {
        setPhase("waiting");
        return;
      }
      timer = setTimeout(poll, 1000);
    };
    timer = setTimeout(poll, 1000);
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [phase]);
  const confirm = async () => {
    setError("");
    setPhase("sending");
    try {
      boot.current = (await api.application()).boot_id;
      await api.controlApplication(action);
      setPhase(action === "restart" ? "restarting" : "stopped");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setPhase("confirm");
    }
  };
  return (
    <Modal
      title={action === "restart" ? "Restart application" : "Stop application"}
      close={close}
    >
      <ErrorText>{error}</ErrorText>
      {phase === "confirm" && (
        <>
          <p>
            {action === "restart"
              ? "Restart scrobbl.rs now? Collection pauses while the archive closes and reopens."
              : "Stop scrobbl.rs now? Collection will stop and this page will become unavailable. You will need Docker or your process manager to start it again."}
          </p>
          <p>
            Your saved archive is retained. Your host and other containers stay
            running.
          </p>
          <div className="form-actions">
            <button className="secondary" onClick={close}>
              Cancel
            </button>
            <button className="primary" onClick={() => void confirm()}>
              {action === "restart" ? "Restart now" : "Stop now"}
            </button>
          </div>
        </>
      )}
      {phase !== "confirm" && (
        <>
          <p role="status">
            {phase === "sending"
              ? "Closing the application safely…"
              : phase === "restarting"
                ? "Restarting scrobbl.rs…"
                : phase === "restarted"
                  ? "scrobbl.rs has restarted. Your archive is ready."
                  : phase === "stopped"
                    ? "Stop accepted. Start scrobbl.rs from Docker or your process manager when you are ready."
                    : "The application has not returned yet. Check its logs, then reload this page."}
          </p>
          <button className="secondary" onClick={close}>
            Close
          </button>
        </>
      )}
    </Modal>
  );
}

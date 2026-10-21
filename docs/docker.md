# Install with Docker

The published image runs on **linux/amd64** (x86-64), including Linux hosts and
Docker Desktop using Linux containers. It contains the Rust executable, embedded
React interface, source bundle and notices. Node and Rust are not needed on the
host. Use Docker Compose or paste the same service configuration into a Docker
stack manager. No particular NAS, proxy, VPN or DNS provider is required.

## Choose the image and data directory

Use `ghcr.io/itsmrma/scrobbl-rs:sha-FULL_COMMIT` from a successful **Docker** Actions
run. `:main` follows tested main-branch commits; version tags match Git tags. Pin a
commit or digest for deliberate updates. Publishing never updates your running
container automatically. Feature-branch commit images are development builds,
not a declaration that the first release is complete.

The GHCR package must be public for anonymous pulls. If a pull reports `denied`,
check the package's visibility and the exact published tag; public repository
visibility does not automatically make a new GHCR package public.

Create a **new, dedicated directory on a local filesystem** for `/data`. It holds
`archive.db`, `private/auth.db`, the persistent upload spool and technical upgrade
copies. Keep all of it across container replacements. Do not use an SMB/NFS mount
for the active SQLite database or run two app instances against the same directory.
Do not point this build at the prototype's data directory: reimport into a new
archive. The app refuses to open the prototype database.

The default container identity is UID/GID **10001:10001**. On a Linux host, for example:

```sh
sudo install -d -o 10001 -g 10001 -m 0700 /srv/scrobbl/data
```

Use a path appropriate to your machine. Alternatively, set `SCROBBL_UID` and
`SCROBBL_GID` to a non-root identity with write access to that directory and adjust
its ownership/ACL. On Docker Desktop, use a shared host path; if bind permissions
cannot express a Linux UID, use a Docker-managed volume and initialize its ownership
once using the image with `--user 0:0 --entrypoint chown`. Do not solve a permission
error by running the application as root or making private data world-readable.
The image works with either bind mounts or named volumes; the supplied Compose
uses a bind mount to make the data location explicit. For a named volume, replace
that mount with `scrobbl-data:/data` and declare `volumes: {scrobbl-data: {}}`.

## Start the generic Compose stack

Download [compose.yaml](../compose.yaml) and create a private `.env` next to it.
This example uses documentation-only addresses: replace all values with yours.

```dotenv
SCROBBL_IMAGE=ghcr.io/itsmrma/scrobbl-rs:sha-FULL_COMMIT
SCROBBL_DATA_PATH=/srv/scrobbl/data
SCROBBL_HOST_IP=192.0.2.10
SCROBBL_HOST_PORT=3210
SCROBBL_EXTERNAL_ORIGIN=http://192.0.2.10:3210
# Optional permission override:
# SCROBBL_UID=10001
# SCROBBL_GID=10001
```

```sh
docker compose config --quiet
docker compose pull
docker compose up -d
docker compose ps
docker compose logs --tail=100 scrobbl
```

In a stack manager, supply those variables in its environment editor and paste the
Compose file. The bind directory must already exist on the Docker host. Port
mapping uses the **private host IP** you choose; choose an unused port and check
host firewall rules. `127.0.0.1` exposes it only on the Docker host, suitable for a
proxy running on that host. A proxy in a different container cannot use its own
`127.0.0.1` to reach the app.

Open the configured origin and create the owner password (at least 12 characters).
The health check indicates HTTP/database availability; Spotify status is separate.
The read-only root filesystem and dropped capabilities in Compose are supported;
only `/data` and the temporary directory need writes. `SIGTERM` shuts down the
service and closes database pools before container replacement.

Private HTTP supports browsing and imports. To connect Spotify on a server, finish
HTTPS below: Spotify only permits plain HTTP callbacks on explicit loopback IPs.
A VPN provides reachability; it does not change the registered callback requirement.
See [Spotify's redirect rules](https://developer.spotify.com/documentation/web-api/concepts/redirect_uri).

## Configure a private HTTPS reverse proxy

Collect these values before configuring any proxy:

| Value | Example | Used by |
| --- | --- | --- |
| DNS name you control | `music.example.org` or `myname.duckdns.org` | Browser, certificate, Spotify |
| Proxy's reachable private IP | `192.0.2.20` | LAN/VPN DNS resolution |
| External TLS port | `443` or an available private port such as `8443` | Proxy listener and origin |
| External origin | `https://music.example.org:8443` | App configuration and browser |
| Internal upstream | `http://HOST_PRIVATE_IP:3210` or `http://scrobbl:3210` | Proxy's HTTP destination |
| Persistent certificate storage | A volume/directory managed by the proxy | Certificates, ACME account and renewal |
| DNS API credentials | Scoped token for the authoritative DNS provider | Certificate manager only |

1. **Choose a name.** Without a domain, register a free name at
   [DuckDNS](https://www.duckdns.org/). With a domain, choose a subdomain under a DNS
   provider with automated certificate support. The registrar and authoritative
   DNS provider may differ.
2. **Issue a trusted certificate through DNS-01.** Configure your certificate
   manager's integration for DuckDNS or your provider. It must create the public
   `_acme-challenge` TXT record, wait for propagation and renew automatically.
   Persist its account/certificate data. Keep DNS credentials there, outside
   scrobbl.rs. Check whether its DuckDNS integration changes address records;
   private resolution should remain under your control. DNS-01 can validate
   services that are not reachable from the public Internet; no router port
   forwarding is required. See [Let's Encrypt DNS-01](https://letsencrypt.org/docs/challenge-types/#dns-01-challenge).
3. **Resolve the name privately.** Configure your LAN resolver to return the
   proxy's private IP for that exact name (for example a DNS rewrite). Configure
   VPN clients' DNS and subnet routes so the same name/IP works remotely.
   Browser encrypted-DNS settings must not bypass the private resolver. The
   certificate must cover the DNS name, even though it resolves to a private IP.
4. **Connect proxy to app.** Set a TLS listener on an available private port and
   the HTTP upstream from the table. Preserve the complete path/query and
   **external Host header including any nonstandard port**. Allow uploads up to
   your configured app limit plus multipart overhead; use streaming uploads and
   enough proxy timeout for receipt. Import processing continues in the background.
   Do not rewrite `/auth/spotify/callback` or mount the app under a path prefix.
5. **Restrict upstream access.** Choose either a private published host port
   reachable by the proxy, or a shared Docker network. For a container proxy,
   remove `ports` from the app service, join both services to the same explicitly
   named Docker network and use `http://scrobbl:3210` as upstream. The proxy alone
   publishes its TLS listener. A published app port must be limited by your
   private network/firewall; it is not an additional browser-facing origin.
6. **Configure the app.** Set `SCROBBL_EXTERNAL_ORIGIN` to the exact HTTPS origin
   and recreate the app container. `SCROBBL_BIND=0.0.0.0` and `SCROBBL_PORT=3210`
   refer to the internal listener, independently of the external TLS port.
   Open the HTTPS name, verify the certificate and sign in there.
7. **Connect Spotify.** In Settings → Connections, copy the displayed redirect
   URI into the Spotify Developer Dashboard, save app credentials in scrobbl.rs
   and choose Connect Spotify. Scheme, name, optional port and callback path must
   match. The browser returns directly to the same signed-in HTTPS origin;
   no browser helper, SSH tunnel or copied tokens are needed. See
   [Spotify setup](spotify-setup.md) for app access and diagnostics.
8. **Verify renewal and persistence.** Check the manager's renewal schedule,
   provider permissions and last result, certificate expiry and full chain.
   After a controlled renewal, verify the served certificate again from LAN/VPN.
   A staging challenge or a renew command alone does not demonstrate that the
   browser receives the renewed certificate.

The app trusts **no forwarding headers**: `Forwarded`, `X-Forwarded-Host`,
`X-Forwarded-Proto` and client-IP headers are ignored. There is no trusted-header
allowlist to maintain. The configured origin alone governs Host/Origin validation,
callback construction and `Secure` cookies. TLS terminates at the proxy; the HTTP
upstream must remain in the operator's trusted private network. A proxy that
rewrites Host to its internal upstream will receive a 403 and must be configured
to preserve the external Host. Do not weaken CSRF or add wildcard origins.

## Startup configuration

Compose variables such as `SCROBBL_HOST_PORT`, `SCROBBL_DATA_PATH`, `SCROBBL_IMAGE`
and UID/GID are host-side choices. Inside the app:

| Variable | Default outside Docker | Meaning |
| --- | --- | --- |
| `SCROBBL_BIND` | `127.0.0.1` | Internal listening IP; the image uses `0.0.0.0` |
| `SCROBBL_PORT` | `3210` | Internal HTTP port |
| `SCROBBL_EXTERNAL_ORIGIN` | Loopback listener origin | Browser-facing HTTP(S) origin, no path/query/credentials; required for non-loopback binds |
| `SCROBBL_DATA_DIR` | `./data` | Persistent root; the image uses `/data` |

The bare image defaults to `http://127.0.0.1:3210` for a local Docker quick start;
set the external origin for all other access. The configured origin is the only
accepted browser origin. Changing it requires a restart and signing in at the new
address, plus updating Spotify's registered callback. Other import/polling and
optional Spotify credential variables are listed in [README](../README.md).
For Compose, add extra app variables explicitly to the service's `environment`;
a host `.env` variable alone is not automatically passed into the container.
Spotify app credentials and preferences normally belong in Settings. When both
Spotify credential environment variables are supplied they override the saved pair.

## Updates and recovery boundaries

Select a new tested commit/version, stop the app cleanly and preserve a copy of the
**entire** stopped data directory plus your Compose/environment configuration in a
private location. Then change `SCROBBL_IMAGE`, pull and recreate manually. Keep the
previous image/digest. A copy of only `archive.db` while SQLite is active is not a
backup; never remove volumes as part of an update. Startup creates verified
technical safety copies before schema changes; see
[archive foundations](archive-foundations.md) for rollback boundaries. Downgrading
an image alone is not a database rollback. The complete portable backup/restore
flow and failure testing belong to the release's recovery work and are not
claimed by this installation guide.

## Troubleshooting and acceptance

| Symptom | Check |
| --- | --- |
| Startup permission error | Host path exists, local filesystem, UID/GID/ACL, directory writable by container identity |
| App returns 403 | Browser origin matches configuration; proxy preserved external Host and port; HTTP and HTTPS are different origins |
| Browser certificate error | DNS destination, certificate name/expiry, full chain, system time; fix before signing in |
| OAuth callback missing session/state | Sign in at the HTTPS origin, start Connect again, use the same browser; do not reuse callbacks or change host mid-flow |
| Spotify refuses callback | Register the exact URI from Connections; LAN HTTP is unsupported by Spotify |
| Renewal failure | Certificate manager logs, DNS permissions/propagation, account storage and served certificate |
| Reachable in LAN, not VPN | Private DNS resolution, subnet routing, firewall and listener binding |
| Unhealthy container | App logs, mount access and configured Host; health does not test Spotify authorization |

Before calling an installation verified, record actual evidence for Firefox on
PC/Mac and Safari on iPhone/iPad: private DNS/chain, login and callback, canceled
and restarted authorization, reconnect, refresh, collection with browser closed,
now-playing source/request/display timing, container restart/recreation with
archive and credentials retained, and certificate renewal. Automated simulated
tests are separate evidence and do not establish these live results.

## Build and corresponding source

From the repository, `docker build --platform linux/amd64 --build-arg VCS_REF=FULL_COMMIT
-t scrobbl-local .` compiles frontend and backend using the locks. For source
extracted from an image, unpack `source.tar.gz`, enter `scrobbl-rs`, use the Rust
version recorded in `distribution.json` and Node 24, then run `npm run build` and
`cargo build --locked --offline --release`. npm build inputs and Cargo sources are
already included; `.cargo/config.toml` selects the vendored registry. To rebuild
through Docker, its build stages install npm dependencies again and need network
access. The source archive and notices are also attached to the matching Docker
Actions run and version-release draft; see [third-party notices](../THIRD_PARTY_NOTICES.md).

The Docker workflow builds and loads the image, runs the generic Compose smoke
(import, health, non-root/read-only mode, persistent owner/session/preferences/data,
recreation and graceful stop), then publishes that tested image. PR builds exercise
the same image without publishing. Image contents include exact source/notices and
`--version` reports the linked SQLite version/source ID for artifact verification.

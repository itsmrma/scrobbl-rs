# Pull request checks and Docker publication

`Checks` runs for every PR targeting `main` on opening, a new commit, reopening
and becoming ready for review, including drafts. Pushes to `main` and `v*` tags
run the same verification. A detection job skips the heavy jobs when every changed
path is under `docs/` or ends in `.md`; version tags always run the full suite.
The workflow itself always runs, so required checks are not stranded. A new
PR run cancels the superseded run for that PR; separate PRs and trusted pushes
have separate concurrency groups. Running trusted push workflows are allowed to finish.

## Required checks

The protection on `main` requires these GitHub Actions checks, with the branch
up to date. It also applies to administrators, prohibits force pushes/deletion
and requires a PR with **zero** additional approving reviews:

| Check | Verification |
| --- | --- |
| `Backend (Linux)` | Locked build/tests, rustfmt, Clippy with warnings denied |
| `Backend (Windows)` | The same suite, including process lock/crash recovery, temporary spool and linked SQLite |
| `Browser (chromium)` | Locked npm install, TypeScript, Vite, committed assets, browser tests |
| `Browser (firefox)` | The same frontend checks in Firefox |
| `Browser (webkit)` | The same frontend checks in WebKit |
| `Docker / linux/amd64` | Image build, generic Compose startup and persistent-volume smoke |
| `PR checks` | Always runs; requires successful verification for application changes or the explicit documentation-only skip |

A failed or canceled verification cannot make `PR checks` pass. For application
changes, every verification must succeed; for documentation-only changes, all
three job groups must be skipped and the detector must report `false`. Only the
current PR revision can satisfy required checks. GitHub's up-to-date requirement
also prevents using checks against an obsolete base. Keep these names and the
repository protection synchronized when changing the workflow.

From a PR branch, inspect its current required results with `gh pr checks
--required`. A pending result on a newly pushed commit must complete before
merging, even if an older run succeeded. To retry an infrastructure failure,
use the run's **Re-run failed jobs** action; download its diagnostics first.

The workflows use Rust **1.99.1** (matching Docker) and Node **24**; the pinned
Docker frontend stage uses Node **25**. Both builds must reproduce `static/dist`.
npm caches use `package-lock.json`; Cargo caches use OS/toolchain/`Cargo.lock` and separate
browser/load keys. Jobs have timeouts. Actions are pinned by commit; update the
pin and its version comment together. No separate frontend lint script exists;
TypeScript and the production build are the project's frontend checks.

## Local equivalents

Run from a clean checkout, with Rust 1.99.1, rustfmt/Clippy and Node 24 installed:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --all-targets
cargo test --locked --all-targets -- --nocapture
npm ci
npm run typecheck
npm run build
git diff --exit-code -- static/dist
git ls-files --others --exclude-standard static/dist
npx playwright install --with-deps chromium firefox webkit
cargo build --locked --bin scrobbl-rs
npm run test:e2e -- --browser=chromium
npm run test:e2e -- --browser=firefox
npm run test:e2e -- --browser=webkit
```

The `git ls-files` command must print nothing: new untracked build outputs also
mean committed assets are stale. Browser tests create a temporary archive;
Rust tests use temporary databases and local fake Spotify responses. No NAS,
personal account, listening archive or provider credentials are required.

On a Linux Docker host, build and run the same disposable Compose smoke:

```sh
docker build --platform linux/amd64 --build-arg VCS_REF="$(git rev-parse HEAD)" -t scrobbl-ci:local .
python3 scripts/verify-docker.py scrobbl-ci:local --revision "$(git rev-parse HEAD)"
```

The smoke verifies non-root/read-only operation, import, health, owner/session/
preferences/history after recreation, graceful SIGTERM, matching source/notices,
the image revision and the binary's linked SQLite version/source ID. It requires
SQLite 3.51.3 or newer, as does startup; a host's separate `sqlite3` executable
does not establish the library linked into the application.

## Publication and fork boundaries

`docker.yml` is a reusable verification workflow called by `Checks`. It builds
once without registry credentials or write permissions. PRs, including forks,
never publish an image. No `pull_request_target`, privileged PR checkout or
secret inheritance is used. Checkout does not persist its token. Fork runs may
need GitHub's normal first-contributor approval; approval grants no extra secret.

Only successful pushes to this repository's `main` or `v*` tags publish. After
`PR checks` succeeds, a separate job with `packages: write` downloads the exact
tested image from the **same run**, loads it and pushes `sha-FULL_COMMIT` and
either `main` or the Git tag. It does not check out repository code or rebuild.
Feature branch pushes do not publish images. Version tags additionally attach
matching source archives to a draft GitHub release in a separate job with
`contents: write`. Publishing does not update the private server or declare the
first release accepted.

## Failure diagnosis and heavy workloads

Open the failed job's logs first. Backend failures upload `ci-logs/`; browser
failures additionally upload Playwright's HTML report, retained traces and
failure screenshots. Docker smoke logs include Compose container output and
are uploaded on failure. Diagnostics and source/notices artifacts last seven
days; the intermediate tested image lasts one day. Docker build failures remain
available in the action's own build logs/summary.

Download diagnostics from the run's Artifacts section. For a browser trace:

```sh
npx playwright show-trace path/to/trace.zip
```

Do not add real Spotify credentials or private archives to reproduce CI. A
browser failure before server startup is usually a build/startup problem; check
`server-build.log` and the job log before debugging UI assertions. A stale asset
failure requires `npm run build` and committing the generated changes. A lockfile
failure requires deliberately updating and committing the appropriate lockfile.

The two heavy synthetic tests stay ignored in normal PR runs. Use Actions →
**Manual archive load** → **Run workflow** during release validation, or:

```sh
cargo test --locked --test foundations foundation_load -- --ignored --exact --nocapture
cargo test --locked --test spotify_import full_size_spotify_zip -- --ignored --exact --nocapture
```

This manual workflow uploads measurements even on failure and cannot publish.
CI simulation remains separate from the release's live server, browser, Spotify
and export-reconciliation acceptance evidence.

Reference: [GitHub's secure Actions guidance](https://docs.github.com/en/actions/reference/security/secure-use)
and [required checks and protected branches](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches).

# Distribution and third-party notices

Copyright (c) 2026 itsmrma and scrobbl.rs contributors.
Original scrobbl.rs code is licensed under **GPL-3.0-or-later**: GNU General Public
License version 3 or (at your option) any later version. See [LICENSE](LICENSE).
This program is distributed without warranty; see the license for details.

Heavily inspired by [Maloja](https://github.com/krateng/maloja), by krateng.
No Maloja code has been copied into this project.

Each Docker image includes `/usr/share/scrobbl-rs/`:

- `LICENSE`, this notice, and `THIRD_PARTY_LICENSES/`: original upstream license,
  copyright, author and NOTICE files, including native subcomponents such as ring,
  bundled SQLite and the Rust standard library. Both choices of dual licenses are
  preserved; this does not relicense dependencies under the project's license.
- `distribution.json` and `cargo-tree.txt`: locked versions, declared licenses,
  Cargo features and the linux/amd64 normal/build dependency graph. The notice
  collection also covers the broader vendored source and installed build tools.
- `source.tar.gz` and SHA-256: exact application source, frontend, locks, build and
  installation scripts, Cargo vendor sources/configuration and installed npm
  build inputs, associated with the image's full Git commit in `distribution.json`.
- `system-packages.tsv` and `system-licenses/`: exact Debian runtime packages and
  their copyright/license/source provenance documents (including glibc and CA data).
  These system libraries retain their own licenses and distribution notices.

The source and notices can be extracted without running the application:

```sh
docker create --name scrobbl-sources ghcr.io/itsmrma/scrobbl-rs:sha-COMMIT
docker cp scrobbl-sources:/usr/share/scrobbl-rs ./scrobbl-distribution
docker rm scrobbl-sources
```

The same source archive is uploaded to the image's GitHub Actions run. Version
builds attach it to a draft release for the release process to review and publish.
The copy in the image remains available independently of Actions artifact retention.
For source build instructions see [Docker installation](docs/docker.md).

The license collector fails on absent Cargo/npm license metadata or notices.
It copies native subcomponent notices recursively, rather than inferring a license
from the top-level dependency name. Review the generated manifest when changing
locks, features, the target or base images; a license inventory is not a legal opinion.
`ts-rs` and `ts-rs-macros` omit their shared license from the published crates;
[their MIT license](docs/licenses/upstream/ts-rs-MIT.txt) was retrieved from the
[exact upstream source revision](https://github.com/Aleph-Alpha/ts-rs/blob/7182ad8289596097235406b715fa04506443b4ad/LICENSE)
recorded in those crates' `.cargo_vcs_info.json` files. Rolldown's native binding
packages omit standalone notices; the collector verifies their version matches
`rolldown` and preserves that package's MIT and native third-party notices for each binding.

## Frontend assets

React, React DOM and scheduler use their packaged MIT notices. The bundled
Material Icons Outlined font comes from the locked `material-icons` npm package
(Apache-2.0); its license is also kept in
[docs/licenses/material-icons-LICENSE.txt](docs/licenses/material-icons-LICENSE.txt).
The favicon is original project artwork. System fonts are referenced, not bundled.
Spotify artwork is fetched from Spotify at runtime; it is not part of this source
or covered by the project's license. No ownership of third-party music/artwork is claimed.

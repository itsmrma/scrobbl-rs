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
  bundled SQLite, llama.cpp/ggml and the Rust standard library. Both choices of
  dual licenses are preserved; this does not relicense dependencies under the
  project's license.
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
recorded in those crates' `.cargo_vcs_info.json` files. The llama bindings and
their `find_cuda_helper` build dependency omit top-level license files too; the
collector adds the texts listed above. Rolldown's native binding
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

## Optional local inference runtime

The official image compiles a CPU-only inference runtime behind the default
`llama` cargo feature. It includes no model weights and no GPU libraries.

- **llama.cpp and ggml** are bundled as C/C++ sources inside the pinned
  `llama-cpp-sys-2` crate and are compiled into the binary. They are
  **MIT-licensed**; the upstream `llama.cpp/LICENSE` (Copyright (c) 2023-2024
  The ggml authors, Meta Platforms, and contributors) is collected from the
  vendored crate into `THIRD_PARTY_LICENSES/` and the corresponding source is
  included in `source.tar.gz` under `vendor/`.
- **`llama-cpp-2` and `llama-cpp-sys-2`** (the Rust bindings, version 0.1.161,
  upstream revision `e7af607ebd5ec2932709ef918c17cca2f9b0fce4`) are distributed
  under **MIT OR Apache-2.0**, Copyright (c) Dial AI. Their published crates
  omit the top-level license texts, so both choices are kept at
  [docs/licenses/upstream/llama-cpp-rs-MIT.txt](docs/licenses/upstream/llama-cpp-rs-MIT.txt)
  and
  [docs/licenses/upstream/llama-cpp-rs-APACHE-2.0.txt](docs/licenses/upstream/llama-cpp-rs-APACHE-2.0.txt),
  and `scripts/package-distribution.py` copies them into the image notices.
- **`find_cuda_helper` 0.2.0** is an MIT OR Apache-2.0 build-time helper from
  the Rust-CUDA project (`https://github.com/Rust-GPU/Rust-CUDA`) used by
  `llama-cpp-sys-2`. It compiles only the build script, is not linked into the
  application binary, and publishes no license files, so its texts are kept at
  [docs/licenses/upstream/rust-cuda-MIT.txt](docs/licenses/upstream/rust-cuda-MIT.txt)
  and
  [docs/licenses/upstream/rust-cuda-APACHE-2.0.txt](docs/licenses/upstream/rust-cuda-APACHE-2.0.txt).
- The build disables the crate's optional OpenMP feature, so **no libgomp** is
  linked or installed. CUDA, Metal, Vulkan, BLAS and the `common` helper library
  are not built. See [docs/ai.md](docs/ai.md) and [docs/docker.md](docs/docker.md).
- `scripts/package-distribution.py` fails if either the vendored llama.cpp
  license or the bindings' declared license metadata is missing, so the
  inventory cannot silently lose the runtime notices.

## Optional AI models

The official image contains no model weights. Models are downloaded only when
the owner enables AI and starts a download from the curated catalog. Every
catalog entry pins its source repository, revision, file, byte size and
SHA-256; the download is verified before use. The v1 catalog contains **Qwen3
GGUF** models from the Qwen team (Alibaba Group), published under
**Apache-2.0**. Their source repositories, revisions, checksums and attribution
are recorded in [docs/ai.md](docs/ai.md), and the catalog API returns the
license name and URL before a download starts. Model files are not part of this
source distribution, the container image or the Git repository: the owner
downloads them directly from the pinned upstream source after explicit
consent. They remain subject to their upstream license and are not covered by
the project's GPL license.

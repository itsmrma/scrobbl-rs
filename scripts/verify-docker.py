#!/usr/bin/env python3
"""Exercise the built linux/amd64 image and generic Compose on disposable data."""
import argparse
import http.cookiejar
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image")
    parser.add_argument("--revision", required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="scrobbl-docker-") as temporary:
        directory = Path(temporary)
        data = directory / "data"
        data.mkdir()
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        origin = f"http://127.0.0.1:{port}"
        environment = {**os.environ, "SCROBBL_IMAGE": args.image, "SCROBBL_DATA_PATH": str(data),
                       "SCROBBL_EXTERNAL_ORIGIN": origin, "SCROBBL_HOST_IP": "127.0.0.1",
                       "SCROBBL_HOST_PORT": str(port), "SCROBBL_UID": "10001", "SCROBBL_GID": "10001"}
        compose = ["docker", "compose", "-f", str(ROOT / "compose.yaml"), "-p", f"scrobbl-smoke-{port}"]

        def docker(*arguments, capture=False):
            return subprocess.check_output(["docker", *arguments], text=True).strip() if capture else subprocess.check_call(["docker", *arguments])

        def stack(*arguments, capture=False):
            return subprocess.check_output([*compose, *arguments], env=environment, text=True).strip() if capture else subprocess.check_call([*compose, *arguments], env=environment)

        jar = http.cookiejar.CookieJar()
        client = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
        csrf = None

        def request(path, payload=None, content_type="application/json"):
            headers = {"Origin": origin, "Content-Type": content_type}
            if csrf:
                headers["x-csrf-token"] = csrf
            body = payload if isinstance(payload, bytes) else json.dumps(payload).encode() if payload is not None else None
            with client.open(urllib.request.Request(origin + path, data=body, headers=headers), timeout=10) as response:
                return json.load(response)

        def ready():
            for _ in range(90):
                try:
                    return request("/api/session")
                except (OSError, urllib.error.URLError):
                    time.sleep(1)
            raise RuntimeError("Container failed to become ready")

        # Initialize a disposable bind mount using the same numeric identity as the default image.
        docker("run", "--rm", "--user", "0:0", "--entrypoint", "chown", "--volume", f"{data}:/data", args.image, "10001:10001", "/data")
        try:
            stack("config", "--quiet")
            stack("up", "-d", "--pull", "never")
            assert ready()["setup_required"]
            csrf = request("/api/setup", {"password": "disposable Docker smoke password"})["csrf"]
            preferences = request("/api/v1/preferences")
            preferences["name"] = "Docker persistence smoke"
            request("/api/v1/preferences", {"preferences": preferences, "calendar_scope": "future"})
            row = {"ts": "2026-01-01T10:00:00Z", "ms_played": 180000,
                   "master_metadata_track_name": "Container smoke", "master_metadata_album_artist_name": "Synthetic",
                   "master_metadata_album_album_name": "Synthetic", "spotify_track_uri": "spotify:track:0000000000000000000001"}
            boundary = "scrobbl-smoke-boundary"
            multipart = (f'--{boundary}\r\nContent-Disposition: form-data; name="files"; filename="smoke.json"\r\nContent-Type: application/json\r\n\r\n'
                         + json.dumps([row]) + f"\r\n--{boundary}--\r\n").encode()
            request("/api/import/spotify", multipart, f"multipart/form-data; boundary={boundary}")
            for _ in range(60):
                if request("/api/history")["total"] == 1:
                    break
                time.sleep(1)
            else:
                raise RuntimeError("Background import failed")
            container = stack("ps", "-q", "scrobbl", capture=True)
            docker("exec", container, "scrobbl-rs", "--healthcheck")
            print(docker("exec", container, "scrobbl-rs", "--version", capture=True))
            # Recreate, not just restart: authentication, preferences, archive and jobs must survive.
            stack("up", "-d", "--force-recreate", "--pull", "never")
            assert ready()["authenticated"]
            assert request("/api/v1/preferences")["name"] == "Docker persistence smoke"
            assert request("/api/history")["total"] == 1
            assert not request("/api/session")["setup_required"]
            container = stack("ps", "-q", "scrobbl", capture=True)
            inspection = json.loads(docker("inspect", container, capture=True))[0]
            assert inspection["Config"]["User"] == "10001:10001"
            assert inspection["HostConfig"]["ReadonlyRootfs"]
            docker("cp", f"{container}:/usr/share/scrobbl-rs", str(directory / "distribution"))
            manifest = json.loads((directory / "distribution/distribution.json").read_text())
            assert manifest["revision"] == args.revision
            assert any(p["name"] == "libsqlite3-sys" and p["target_build"] for p in manifest["cargo"])
            assert {"react", "react-dom", "material-icons"} <= {p["name"] for p in manifest["npm"]}
            docker("exec", container, "sh", "-c", "cd /usr/share/scrobbl-rs && sha256sum --check source.tar.gz.sha256")
            # Stop via SIGTERM; a SIGKILL fallback produces exit 137, which must not pass.
            stack("stop")
            stopped = json.loads(docker("inspect", container, capture=True))[0]
            assert stopped["State"]["ExitCode"] == 0, stopped["State"]
            print("Docker smoke passed: non-root/read-only, import, health, source notices, recreation persistence and SIGTERM")
        finally:
            stack("logs", "--no-color")
            stack("down")
            # Return the disposable mount to the runner's identity for automatic cleanup.
            docker("run", "--rm", "--user", "0:0", "--entrypoint", "chown", "--volume", f"{data}:/data", args.image,
                   "-R", f"{os.getuid()}:{os.getgid()}", "/data")


if __name__ == "__main__":
    main()

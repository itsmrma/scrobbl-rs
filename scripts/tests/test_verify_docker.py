"""Unit tests for Docker container layer verification rules."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "verify_docker", ROOT / "scripts/verify-docker.py"
)
assert SPEC and SPEC.loader
verify_docker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verify_docker)


class VerifyDockerDiffTests(unittest.TestCase):
    def test_daemon_init_injection_is_allowed(self):
        diff = [
            "C /usr",
            "C /usr/sbin",
            "A /usr/sbin/docker-init",
        ]
        self.assertEqual(verify_docker.unexpected_diff_entries(diff), [])

    def test_daemon_init_injection_variants_are_allowed(self):
        diff = [
            "C /sbin",
            "A /sbin/docker-init",
            "A /dev/init",
        ]
        self.assertEqual(verify_docker.unexpected_diff_entries(diff), [])

    def test_mounts_data_and_tmp_are_allowed(self):
        diff = [
            "C /data",
            "A /data/models/test.gguf",
            "D /data/old.db",
            "C /tmp",
            "A /tmp/test-file",
        ]
        self.assertEqual(verify_docker.unexpected_diff_entries(diff), [])

    def test_writes_to_system_directories_are_flagged(self):
        diff = [
            "C /usr",
            "A /usr/bin/unexpected-binary",
            "C /etc",
            "A /etc/modified.conf",
            "A /app/leak.txt",
            "A /root/.cache",
        ]
        self.assertEqual(
            verify_docker.unexpected_diff_entries(diff),
            [
                "A /usr/bin/unexpected-binary",
                "C /etc",
                "A /etc/modified.conf",
                "A /app/leak.txt",
                "A /root/.cache",
            ],
        )

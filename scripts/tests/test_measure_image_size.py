"""Docker-free tests for the #180 distribution budget rules."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "measure_image_size", ROOT / "scripts/measure-image-size.py"
)
assert SPEC and SPEC.loader
budget = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(budget)

MIB = 1024 * 1024


class EvaluateTests(unittest.TestCase):
    def test_runtime_delta_excludes_the_source_bundle(self):
        result = budget.evaluate(
            image_bytes=200 * MIB,
            baseline_bytes=100 * MIB,
            distribution_bytes=80 * MIB,
            binary_bytes=20 * MIB,
        )
        self.assertEqual(result["raw_image_delta_bytes"], 100 * MIB)
        self.assertEqual(result["runtime_image_delta_bytes"], 20 * MIB)
        self.assertTrue(result["image_ok"])
        self.assertTrue(result["binary_ok"])
        self.assertTrue(result["ok"])

    def test_runtime_delta_over_budget_fails(self):
        result = budget.evaluate(
            image_bytes=200 * MIB,
            baseline_bytes=100 * MIB,
            distribution_bytes=70 * MIB,
            binary_bytes=20 * MIB,
        )
        self.assertEqual(result["runtime_image_delta_bytes"], 30 * MIB)
        self.assertFalse(result["image_ok"])
        self.assertFalse(result["ok"])

    def test_binary_over_budget_fails(self):
        result = budget.evaluate(
            image_bytes=None,
            baseline_bytes=None,
            distribution_bytes=0,
            binary_bytes=31 * MIB,
        )
        self.assertIsNone(result["image_ok"])
        self.assertFalse(result["binary_ok"])
        self.assertFalse(result["ok"])

    def test_exact_budget_boundaries_pass(self):
        result = budget.evaluate(
            image_bytes=125 * MIB,
            baseline_bytes=100 * MIB,
            distribution_bytes=0,
            binary_bytes=30 * MIB,
        )
        self.assertTrue(result["image_ok"])
        self.assertTrue(result["binary_ok"])
        self.assertTrue(result["ok"])

    def test_image_requires_a_baseline(self):
        with self.assertRaises(ValueError):
            budget.evaluate(
                image_bytes=1,
                baseline_bytes=None,
                distribution_bytes=0,
                binary_bytes=None,
            )

    def test_negative_distribution_is_rejected(self):
        with self.assertRaises(ValueError):
            budget.evaluate(
                image_bytes=1,
                baseline_bytes=0,
                distribution_bytes=-1,
                binary_bytes=None,
            )


if __name__ == "__main__":
    unittest.main()

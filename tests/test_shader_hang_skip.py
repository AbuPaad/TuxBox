"""A compiler that hangs must not cost a timeout on every boot.

`klbf-lab` skips one shader by hash because it locks the AMD compiler. Our
preparation already skips a hash that fails to compile, because the worker
writes an `.unsupported` marker beside it. A hang never reaches that path: the
tool does not return, so there is no error to remember. Each boot pays the
timeout again.
"""
import subprocess
import sys
import unittest
from pathlib import Path

from project_tuxbox.runtime_shaders import (
    HANG_LIMIT,
    exact_marker,
    note_hang,
    remembered_reason,
)
from project_tuxbox.shader_prep import ToolFailure, run_tool

KEY = "a" * 64


class HangCounter(unittest.TestCase):
    def setUp(self):
        import tempfile
        self.dir = Path(tempfile.mkdtemp())
        self.dest = self.dir / (KEY + ".cso")

    def test_one_timeout_only_counts(self):
        self.assertIsNone(note_hang(self.dest, KEY, 30))
        self.assertEqual((self.dir / (KEY + ".timeouts")).read_text().strip(), "1")
        self.assertFalse((self.dir / (KEY + ".unsupported")).exists())

    def test_a_second_timeout_publishes_a_refusal(self):
        self.assertIsNone(note_hang(self.dest, KEY, 30))
        reason = note_hang(self.dest, KEY, 30)
        self.assertIsNotNone(reason)
        self.assertIn("timed out", reason)
        self.assertEqual(HANG_LIMIT, 2)

    def test_the_published_marker_short_circuits_the_next_attempt(self):
        note_hang(self.dest, KEY, 30)
        note_hang(self.dest, KEY, 30)
        marker = self.dir / (KEY + ".unsupported")
        self.assertTrue(exact_marker(marker, KEY.encode("ascii")))
        self.assertIn("timed out", remembered_reason(self.dir / (KEY + ".reason")))

    def test_the_counter_resets_after_publication(self):
        note_hang(self.dest, KEY, 30)
        note_hang(self.dest, KEY, 30)
        self.assertFalse((self.dir / (KEY + ".timeouts")).exists())


class TimeoutReporting(unittest.TestCase):
    def test_run_tool_names_a_timeout(self):
        with self.assertRaises(ToolFailure) as caught:
            run_tool([sys.executable, "-c", "import time; time.sleep(5)"], timeout=0.2)
        self.assertTrue(caught.exception.timed_out)

    def test_an_exit_code_is_not_a_timeout(self):
        with self.assertRaises(ToolFailure) as caught:
            run_tool([sys.executable, "-c", "raise SystemExit(3)"])
        self.assertFalse(caught.exception.timed_out)


if __name__ == "__main__":
    unittest.main()

"""The lanes package must be built from the targets that produce its files.

`python build.py lanes` copied `build/bvh/XboxBvhBuilder.dll` while the lanes
target list did not include `bvh`, so a clean checkout failed at the last step,
after every expensive build had succeeded.
"""
import unittest

import build


class LanesTargets(unittest.TestCase):
    def test_build_declares_the_lanes_tables(self):
        self.assertTrue(hasattr(build, "LANES_TARGETS"), "build.py must declare LANES_TARGETS")
        self.assertTrue(hasattr(build, "LANES_FILES"), "build.py must declare LANES_FILES")

    def test_every_copied_file_has_a_built_target(self):
        wanted = {target for target, _ in build.LANES_FILES}
        self.assertLessEqual(wanted, set(build.LANES_TARGETS))

    def test_lanes_still_builds_its_own_target(self):
        self.assertIn("lanes", build.LANES_TARGETS)

    def test_the_bvh_dll_is_shipped(self):
        names = {path for _, path in build.LANES_FILES}
        self.assertIn("bvh/XboxBvhBuilder.dll", names)

    def test_lanes_builds_last(self):
        # The link step needs every provider library, so lanes comes last.
        self.assertEqual(build.LANES_TARGETS[-1], "lanes")


if __name__ == "__main__":
    unittest.main()

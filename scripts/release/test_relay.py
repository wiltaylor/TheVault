"""Small real Docker transport test, not an application release build.

Run only on the card's Docker VM. Uses no signing or publishing credentials.
The unittest result emits TAP directly for terrarium test push.
"""
import os
from pathlib import Path
import sys
import tempfile
import traceback
import unittest
from unittest.mock import patch

from local import identity
from release import Release


class RelayTests(unittest.TestCase):
    def test_owned_image_container_copy_and_cleanup(self):
        with tempfile.TemporaryDirectory(prefix="thevault-release-relay-test-") as directory:
            root = Path(directory)
            release = identity("a" * 40, "0.0.1-alpha.1", "release/0.0.1", "pre-release")
            with patch.dict(os.environ, {"DOCKER_HOST": "unix:///var/run/docker.sock"}):
                job = Release(root / "state", release, root / "runtime")
            image = job.prefix + "-fixture:test"
            context = root / "context"
            context.mkdir()
            (context / "Dockerfile").write_text("FROM alpine:3.22\nRUN mkdir /input /output\n")
            payload = root / "payload.txt"
            payload.write_text("release-relay-copy-proof\n")
            output = root / "output"
            try:
                job.docker("build", "--tag", image, str(context))
                job.container("transport", image,
                              ["sh", "-c", "test -z \"${GH_TOKEN:-}\"; cp /input/payload.txt /output/proof.txt"],
                              copies=[(payload, "/input/payload.txt")], outputs=[("/output/.", output)])
                self.assertEqual((output / "proof.txt").read_text(), payload.read_text())
                self.assertNotEqual(job.docker("inspect", job.prefix + "-transport", check=False).returncode, 0)
            finally:
                job.clean_container(job.prefix + "-transport")
                job.docker("image", "rm", image, check=False)


class TapResult(unittest.TestResult):
    def startTest(self, test):
        super().startTest(test)
        self.number = self.testsRun

    def addSuccess(self, test):
        super().addSuccess(test)
        print(f"ok {self.number} - {test.id()}", flush=True)

    def addFailure(self, test, err):
        super().addFailure(test, err)
        self.report_failure(test, err)

    def addError(self, test, err):
        super().addError(test, err)
        self.report_failure(test, err)

    def report_failure(self, test, err):
        print(f"not ok {self.number} - {test.id()}")
        for line in "".join(traceback.format_exception(*err)).splitlines():
            print("# " + line)


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(RelayTests)
    print("TAP version 13")
    print(f"1..{suite.countTestCases()}")
    result = TapResult()
    suite.run(result)
    sys.exit(0 if result.wasSuccessful() else 1)

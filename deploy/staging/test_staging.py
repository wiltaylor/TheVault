"""Targeted supervisor contract tests. Run with pytest for an original JUnit report."""
import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import Mock

import staging


class Contracts(unittest.TestCase):
    def test_account_creates_key_without_returning_it(self):
        api = Mock()
        api.send.side_effect = [{"accounts": []}, {"id": "acct_fake"}, {}]
        result = staging.account(api, "fake", "anthropic", "http://127.0.0.1:42", "synthetic", "fake")
        self.assertEqual(result, "acct_fake")
        self.assertEqual(api.send.call_args.args,
                         ("PUT", "/api/integrations/ai/accounts/acct_fake/key", {"key": "synthetic"}))

    def test_account_reuses_named_account(self):
        api = Mock()
        api.send.side_effect = [{"accounts": [{"name": "fake", "id": "acct_old"}]}, {}, {}]
        self.assertEqual(staging.account(api, "fake", "anthropic", "http://127.0.0.1:42", "key", "fake"), "acct_old")
        self.assertEqual(api.send.call_args_list[1].args[0], "PUT")

    def test_compose_has_no_network_and_one_capped_service(self):
        source = Path(__file__).with_name("compose.yaml").read_text()
        self.assertIn("network_mode: none", source)
        self.assertEqual(source.count("mem_limit:"), 1)
        self.assertIn("mem_limit: 2g", source)
        for forbidden in ("ports:", "network_mode: host", "docker.sock", "privileged:"):
            self.assertNotIn(forbidden, source)

    def test_recreate_has_fixed_project_and_no_global_cleanup(self):
        source = Path(__file__).with_name("recreate.sh").read_text()
        self.assertIn("-p tv-145-staging -f compose.yaml down --volumes", source)
        for forbidden in ("prune", "--filter", "box-test.lock", "rm -rf"):
            self.assertNotIn(forbidden, source)

    def test_decisions_binary_commands(self):
        import os
        binary = os.environ.get("STAGING_DECISIONS_BIN")
        if not binary:
            self.skipTest("set STAGING_DECISIONS_BIN to test built fake")
        result = subprocess.run([binary], input='requests\nnot-json\n{"answers":{}}\n',
                                capture_output=True, text=True, check=True, timeout=10)
        lines = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertTrue(lines[0]["base_url"].startswith("http://127.0.0.1:"))
        self.assertEqual(lines[1], {"count": 0})
        self.assertIn("error", lines[2])
        self.assertEqual(lines[3], {"ok": "push"})


if __name__ == "__main__":
    unittest.main()

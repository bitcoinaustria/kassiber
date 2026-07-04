import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
USES_RE = re.compile(r"^\s*(?:-\s*)?uses:\s+[^@\s]+@([^#\s]+)")
FULL_SHA_RE = re.compile(r"^[0-9a-f]{40}$")


class WorkflowPinTest(unittest.TestCase):
    def test_integration_workflow_actions_are_pinned_to_commit_shas(self):
        workflow = ROOT / ".github" / "workflows" / "integration.yml"
        refs = []
        for line in workflow.read_text(encoding="utf-8").splitlines():
            match = USES_RE.match(line)
            if match:
                refs.append(match.group(1))

        self.assertTrue(refs)
        for ref in refs:
            self.assertRegex(ref, FULL_SHA_RE)


if __name__ == "__main__":
    unittest.main()

import ast
import re
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
WORKFLOW_SUFFIXES = frozenset({".yml", ".yaml"})
USES_LINE_RE = re.compile(
    r"^\s*(?:-\s*)?(?:uses|['\"]uses['\"])\s*:\s*(?P<value>.+?)\s*$"
)
USES_TOKEN_RE = re.compile(r"(?:^|[,{\s])(?:uses|['\"]uses['\"])\s*:")
FULL_SHA_RE = re.compile(r"^[0-9a-f]{40}$")


def _without_yaml_comment(text: str) -> str:
    quote: str | None = None
    escaped = False
    for index, char in enumerate(text):
        if escaped:
            escaped = False
            continue
        if char == "\\" and quote == '"':
            escaped = True
            continue
        if quote:
            if char == quote:
                quote = None
            continue
        if char in {"'", '"'}:
            quote = char
        elif char == "#":
            return text[:index]
    return text


def _uses_scalar(raw_value: str, *, location: str) -> str:
    value = _without_yaml_comment(raw_value).strip()
    if not value:
        raise AssertionError(f"{location}: uses value is empty")
    if value[0] in {"'", '"'}:
        try:
            parsed = ast.literal_eval(value)
        except (SyntaxError, ValueError) as exc:
            raise AssertionError(f"{location}: uses value is not a valid quoted scalar") from exc
        if not isinstance(parsed, str):
            raise AssertionError(f"{location}: uses value must be a string")
        value = parsed
    if any(char.isspace() for char in value):
        raise AssertionError(f"{location}: uses value must be one scalar")
    return value


def collect_workflow_uses(workflows_root: Path) -> list[tuple[str, str]]:
    references: list[tuple[str, str]] = []
    workflow_files = sorted(
        path
        for path in workflows_root.rglob("*")
        if path.is_file() and path.suffix.lower() in WORKFLOW_SUFFIXES
    )
    if not workflow_files:
        raise AssertionError("no GitHub Actions workflow files were found")
    for workflow in workflow_files:
        for line_number, line in enumerate(workflow.read_text(encoding="utf-8").splitlines(), start=1):
            uncommented = _without_yaml_comment(line)
            match = USES_LINE_RE.match(uncommented)
            location = f"{workflow}:{line_number}"
            if match:
                references.append((_uses_scalar(match.group("value"), location=location), location))
            elif USES_TOKEN_RE.search(uncommented):
                raise AssertionError(f"{location}: unsupported uses syntax cannot be audited safely")
    if not references:
        raise AssertionError("no GitHub Actions uses references were found")
    return references


def assert_workflow_uses_are_pinned(workflows_root: Path) -> None:
    for reference, location in collect_workflow_uses(workflows_root):
        if reference.startswith("./"):
            continue
        if reference.startswith("docker://"):
            raise AssertionError(f"{location}: Docker actions are not allowed by the pin policy")
        action, separator, revision = reference.rpartition("@")
        if not separator or not action or not FULL_SHA_RE.fullmatch(revision):
            raise AssertionError(f"{location}: {reference!r} is not pinned to a full commit SHA")


class WorkflowPinTest(unittest.TestCase):
    def test_every_workflow_action_is_pinned_to_a_commit_sha(self):
        assert_workflow_uses_are_pinned(ROOT / ".github" / "workflows")

    def test_quoted_and_secondary_workflow_references_are_checked(self):
        with tempfile.TemporaryDirectory() as temporary:
            workflows = Path(temporary)
            pinned = "a" * 40
            (workflows / "first.yml").write_text(
                f"steps:\n  - uses: 'actions/checkout@{pinned}'\n",
                encoding="utf-8",
            )
            (workflows / "second.yaml").write_text(
                'steps:\n  - "uses": "actions/setup-python@v5"\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(AssertionError, "setup-python@v5"):
                assert_workflow_uses_are_pinned(workflows)

            (workflows / "second.yaml").write_text(
                f'steps:\n  - "uses": "actions/setup-python@{pinned}"\n',
                encoding="utf-8",
            )
            assert_workflow_uses_are_pinned(workflows)

    def test_unsupported_inline_uses_syntax_fails_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            workflows = Path(temporary)
            (workflows / "inline.yml").write_text(
                "steps:\n  - { uses: actions/checkout@v4 }\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(AssertionError, "cannot be audited"):
                collect_workflow_uses(workflows)


if __name__ == "__main__":
    unittest.main()

import re
import tempfile
import unittest
from pathlib import Path

import yaml
from yaml.nodes import MappingNode, Node, ScalarNode, SequenceNode


ROOT = Path(__file__).resolve().parent.parent
WORKFLOW_SUFFIXES = frozenset({".yml", ".yaml"})
FULL_SHA_RE = re.compile(r"^[0-9a-f]{40}$")


def _node_location(workflow: Path, node: Node) -> str:
    return f"{workflow}:{node.start_mark.line + 1}"


def _collect_uses_nodes(
    node: Node,
    *,
    workflow: Path,
    references: list[tuple[str, str]],
) -> None:
    if isinstance(node, SequenceNode):
        for child in node.value:
            _collect_uses_nodes(child, workflow=workflow, references=references)
        return
    if not isinstance(node, MappingNode):
        return
    for key_node, value_node in node.value:
        if not isinstance(key_node, ScalarNode):
            raise AssertionError(
                f"{_node_location(workflow, key_node)}: complex YAML mapping keys cannot be audited safely"
            )
        if key_node.value == "uses":
            location = _node_location(workflow, key_node)
            if not isinstance(value_node, ScalarNode) or value_node.tag != "tag:yaml.org,2002:str":
                raise AssertionError(f"{location}: uses value must be a string")
            reference = value_node.value.strip()
            if not reference or any(char.isspace() for char in reference):
                raise AssertionError(f"{location}: uses value must be one non-empty scalar")
            references.append((reference, location))
        _collect_uses_nodes(value_node, workflow=workflow, references=references)


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
        try:
            documents = yaml.compose_all(
                workflow.read_text(encoding="utf-8"),
                Loader=yaml.SafeLoader,
            )
            for document in documents:
                if document is not None:
                    _collect_uses_nodes(
                        document,
                        workflow=workflow,
                        references=references,
                    )
        except yaml.YAMLError as exc:
            raise AssertionError(f"{workflow}: workflow YAML is invalid") from exc
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

    def test_inline_explicit_and_escaped_uses_keys_are_audited(self):
        with tempfile.TemporaryDirectory() as temporary:
            workflows = Path(temporary)
            pinned = "a" * 40
            workflow = workflows / "adversarial.yml"
            workflow.write_text(
                f"steps:\n"
                f"  - uses: actions/checkout@{pinned}\n"
                "  - ? uses\n"
                "    : actions/setup-python@v5\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(AssertionError, "setup-python@v5"):
                assert_workflow_uses_are_pinned(workflows)

            workflow.write_text(
                f"steps:\n"
                f"  - uses: actions/checkout@{pinned}\n"
                '  - "us\\u0065s": actions/setup-python@v5\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(AssertionError, "setup-python@v5"):
                assert_workflow_uses_are_pinned(workflows)

            workflow.write_text(
                f"steps:\n"
                f"  - uses: actions/checkout@{pinned}\n"
                f"  - {{ uses: actions/setup-python@{pinned} }}\n",
                encoding="utf-8",
            )
            assert_workflow_uses_are_pinned(workflows)


if __name__ == "__main__":
    unittest.main()

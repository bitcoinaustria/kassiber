import re
import tempfile
import unittest
from pathlib import Path

import yaml
from yaml.nodes import MappingNode, Node, ScalarNode, SequenceNode


ROOT = Path(__file__).resolve().parent.parent
WORKFLOW_SUFFIXES = frozenset({".yml", ".yaml"})
FULL_SHA_RE = re.compile(r"^[0-9a-f]{40}$")
PINNED_DOCKER_IMAGE_RE = re.compile(r"^docker://[^@\s]+@sha256:[0-9a-f]{64}$")


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


def _collect_yaml_uses(document_path: Path) -> list[tuple[str, str]]:
    references: list[tuple[str, str]] = []
    try:
        documents = yaml.compose_all(
            document_path.read_text(encoding="utf-8"),
            Loader=yaml.SafeLoader,
        )
        for document in documents:
            if document is not None:
                _collect_uses_nodes(
                    document,
                    workflow=document_path,
                    references=references,
                )
    except yaml.YAMLError as exc:
        raise AssertionError(f"{document_path}: workflow YAML is invalid") from exc
    return references


def _workflow_files(workflows_root: Path) -> list[Path]:
    workflow_files = sorted(
        path
        for path in workflows_root.rglob("*")
        if path.is_file() and path.suffix.lower() in WORKFLOW_SUFFIXES
    )
    if not workflow_files:
        raise AssertionError("no GitHub Actions workflow files were found")
    return workflow_files


def collect_workflow_uses(workflows_root: Path) -> list[tuple[str, str]]:
    references: list[tuple[str, str]] = []
    workflow_files = _workflow_files(workflows_root)
    for workflow in workflow_files:
        references.extend(_collect_yaml_uses(workflow))
    if not references:
        raise AssertionError("no GitHub Actions uses references were found")
    return references


def _default_repository_root(workflows_root: Path) -> Path:
    resolved = workflows_root.resolve()
    if resolved.name == "workflows" and resolved.parent.name == ".github":
        return resolved.parent.parent
    return resolved


def _resolve_local_uses(
    reference: str,
    *,
    location: str,
    repository_root: Path,
) -> Path:
    target = (repository_root / reference[2:]).resolve()
    try:
        target.relative_to(repository_root)
    except ValueError as exc:
        raise AssertionError(f"{location}: local action escapes the repository") from exc
    if target.is_dir():
        candidates = [
            target / name
            for name in ("action.yml", "action.yaml")
            if (target / name).is_file()
        ]
        if len(candidates) != 1:
            raise AssertionError(
                f"{location}: local action must contain exactly one action.yml or action.yaml"
            )
        action_path = candidates[0].resolve()
        try:
            metadata = yaml.safe_load(action_path.read_text(encoding="utf-8"))
        except yaml.YAMLError as exc:
            raise AssertionError(f"{action_path}: local action YAML is invalid") from exc
        if not isinstance(metadata, dict):
            raise AssertionError(f"{action_path}: local action metadata must be a mapping")
        runs = metadata.get("runs")
        if isinstance(runs, dict) and str(runs.get("using") or "").lower() == "docker":
            image = runs.get("image")
            normalized_image = image.strip() if isinstance(image, str) else ""
            if (
                normalized_image.lower().startswith("docker://")
                and not PINNED_DOCKER_IMAGE_RE.fullmatch(normalized_image)
            ):
                raise AssertionError(
                    f"{location}: local Docker action image {image!r} is not pinned to a sha256 digest"
                )
        return action_path
    if target.is_file() and target.suffix.lower() in WORKFLOW_SUFFIXES:
        return target
    raise AssertionError(f"{location}: local action or workflow was not found")


def assert_workflow_uses_are_pinned(
    workflows_root: Path,
    *,
    repository_root: Path | None = None,
) -> None:
    repository_root = (repository_root or _default_repository_root(workflows_root)).resolve()
    pending = _workflow_files(workflows_root)
    visited: set[Path] = set()
    reference_count = 0
    while pending:
        document_path = pending.pop()
        resolved_document = document_path.resolve()
        try:
            resolved_document.relative_to(repository_root)
        except ValueError as exc:
            raise AssertionError(
                f"{document_path}: audited YAML escapes the repository"
            ) from exc
        if resolved_document in visited:
            continue
        visited.add(resolved_document)
        for reference, location in _collect_yaml_uses(resolved_document):
            reference_count += 1
            if reference.startswith("./"):
                pending.append(
                    _resolve_local_uses(
                        reference,
                        location=location,
                        repository_root=repository_root,
                    )
                )
                continue
            if reference.startswith("docker://"):
                raise AssertionError(
                    f"{location}: Docker actions are not allowed by the pin policy"
                )
            action, separator, revision = reference.rpartition("@")
            if not separator or not action or not FULL_SHA_RE.fullmatch(revision):
                raise AssertionError(
                    f"{location}: {reference!r} is not pinned to a full commit SHA"
                )
    if not reference_count:
        raise AssertionError("no GitHub Actions uses references were found")


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

    def test_repository_local_composite_actions_are_audited_recursively(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary)
            workflows = repository / ".github" / "workflows"
            outer = repository / ".github" / "actions" / "outer"
            inner = repository / ".github" / "actions" / "inner"
            workflows.mkdir(parents=True)
            outer.mkdir(parents=True)
            inner.mkdir(parents=True)
            (workflows / "ci.yml").write_text(
                "steps:\n  - uses: ./.github/actions/outer\n",
                encoding="utf-8",
            )
            (outer / "action.yml").write_text(
                "runs:\n  using: composite\n  steps:\n"
                "    - uses: ./.github/actions/inner\n",
                encoding="utf-8",
            )
            (inner / "action.yaml").write_text(
                "runs:\n  using: composite\n  steps:\n"
                "    - uses: actions/setup-python@v5\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(AssertionError, "setup-python@v5"):
                assert_workflow_uses_are_pinned(workflows)

            pinned = "a" * 40
            (inner / "action.yaml").write_text(
                "runs:\n  using: composite\n  steps:\n"
                f"    - uses: actions/setup-python@{pinned}\n"
                "    - uses: ./.github/actions/outer\n",
                encoding="utf-8",
            )
            assert_workflow_uses_are_pinned(workflows)

    def test_repository_local_action_paths_cannot_escape(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "repo"
            workflows = repository / ".github" / "workflows"
            workflows.mkdir(parents=True)
            (workflows / "ci.yml").write_text(
                "steps:\n  - uses: ./../outside-action\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(AssertionError, "escapes the repository"):
                assert_workflow_uses_are_pinned(workflows)

    def test_repository_local_docker_actions_require_an_immutable_remote_image(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary)
            workflows = repository / ".github" / "workflows"
            action = repository / ".github" / "actions" / "docker"
            workflows.mkdir(parents=True)
            action.mkdir(parents=True)
            (workflows / "ci.yml").write_text(
                "steps:\n  - uses: ./.github/actions/docker\n",
                encoding="utf-8",
            )
            metadata = action / "action.yml"
            metadata.write_text(
                "runs:\n  using: docker\n  image: docker://alpine:latest\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(AssertionError, "not pinned to a sha256 digest"):
                assert_workflow_uses_are_pinned(workflows)

            metadata.write_text(
                "runs:\n  using: docker\n  image: DOCKER://alpine:latest\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(AssertionError, "not pinned to a sha256 digest"):
                assert_workflow_uses_are_pinned(workflows)

            metadata.write_text(
                "runs:\n  using: docker\n"
                f"  image: docker://alpine@sha256:{'a' * 64}\n",
                encoding="utf-8",
            )
            assert_workflow_uses_are_pinned(workflows)


if __name__ == "__main__":
    unittest.main()

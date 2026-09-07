"""Deterministic components for explicit analysis hypothesis edges.

This is graph composition only. Callers choose the evidence and observer first;
joining vertices never gives these components ownership/accounting authority.
"""
from __future__ import annotations

from collections import defaultdict
from typing import Iterable


class Components:
    def __init__(self, node_ids: Iterable[str]):
        self.parent = {node_id: node_id for node_id in node_ids}

    def find(self, node_id: str) -> str:
        root = node_id
        while self.parent[root] != root:
            root = self.parent[root]
        while self.parent[node_id] != node_id:
            parent = self.parent[node_id]
            self.parent[node_id] = root
            node_id = parent
        return root

    def union(self, left: str, right: str) -> bool:
        left_root, right_root = self.find(left), self.find(right)
        if left_root == right_root:
            return False
        self.parent[max(left_root, right_root)] = min(left_root, right_root)
        return True

    def groups(self, *, include_singletons: bool = False) -> tuple[tuple[str, ...], ...]:
        groups: dict[str, list[str]] = defaultdict(list)
        for node_id in sorted(self.parent):
            groups[self.find(node_id)].append(node_id)
        return tuple(sorted(tuple(members) for members in groups.values()
                            if include_singletons or len(members) > 1))

"""Loops in a dependency graph: strongly connected components, iteratively (no recursion limit)."""

from __future__ import annotations

from typing import Iterable

# node -> its links: (target node, file, JSON pointer of the member that makes the link)
Links = dict[str, list[tuple[str, str, str]]]


def loops(nodes: Iterable[str], links: Links) -> list[tuple[list[str], str, str]]:
    """Each loop as (its nodes sorted, file, pointer of the first node's first link into it), sorted."""
    targets = {node: [target for target, _, _ in out] for node, out in links.items()}
    found = []
    for component in _components(nodes, targets):
        members = sorted(component)
        inside = set(component)
        into = [link for link in links.get(members[0], ()) if link[0] in inside]
        if not into:  # a single node without a link to itself
            continue
        _, path, ref = into[0]
        found.append((members, path, ref))
    return sorted(found, key=lambda loop: loop[0])


def _components(nodes: Iterable[str], targets: dict[str, list[str]]) -> list[list[str]]:
    """Tarjan's strongly connected components."""
    index: dict[str, int] = {}
    low: dict[str, int] = {}
    stack: list[str] = []
    on_stack: set[str] = set()
    components: list[list[str]] = []

    def visit(node: str) -> None:
        index[node] = low[node] = len(index)
        stack.append(node)
        on_stack.add(node)

    for root in nodes:
        if root in index:
            continue
        visit(root)
        work = [(root, iter(targets.get(root, ())))]
        while work:
            node, children = work[-1]
            for child in children:
                if child not in index:
                    visit(child)
                    work.append((child, iter(targets.get(child, ()))))
                    break
                if child in on_stack:
                    low[node] = min(low[node], index[child])
            else:
                work.pop()
                if work:
                    parent = work[-1][0]
                    low[parent] = min(low[parent], low[node])
                if low[node] == index[node]:
                    component = []
                    while True:
                        member = stack.pop()
                        on_stack.discard(member)
                        component.append(member)
                        if member == node:
                            break
                    components.append(component)
    return components

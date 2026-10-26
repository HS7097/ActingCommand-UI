"""What the run's type check (`types`) and the derivation graph (`graph`) share."""

from __future__ import annotations

from typing import Any, Callable, Hashable, Iterable

from acmodel.check.checker import Checker, Entry
from acmodel.expr import BOOL, NUMBER, TIME, Type, amount

CONFIRMED = "confirmed"
PROVISIONAL = "provisional"
BLOCKED = "blocked"
RANK = {CONFIRMED: 0, PROVISIONAL: 1, BLOCKED: 2}
# A pending rule is usable only through an unknown with a range (the static check makes sure it names
# one), so it behaves as provisional.
RULE_STATUS = {"confirmed": CONFIRMED, "provisional": PROVISIONAL, "pending": PROVISIONAL}
# The rules that have a value, which `rule:<id>` reads: a derive's value, a cap's value, a limit's max, and
# a random rule's (`has_value`): a pity chain's expected pulls or a drop's chance, never `outcomes`.
VALUE_RULES = ("derive", "cap", "limit")
RANDOM_VALUES = ("pity_chain", "drop")


def has_value(rule: Entry) -> bool:
    """Whether a rule has a value that `rule:<id>` reads and the run shows."""
    if rule.kind == "random":
        return any(form in rule.doc["logic"] for form in RANDOM_VALUES)
    return rule.kind in VALUE_RULES


def value_member(rule: Entry) -> tuple[str, ...]:
    """The member of a value rule's `logic` that its value comes from: a limit's `max`, a random rule's form,
    or `value`."""
    if rule.kind == "limit":
        return ("max",)
    if rule.kind == "random":
        return (next(form for form in RANDOM_VALUES if form in rule.doc["logic"]),)
    return ("value",)


def worst(*statuses: str) -> str:
    return max(statuses, key=RANK.__getitem__)


class Need(Exception):
    """A computation needs `key` first."""

    def __init__(self, key: Hashable) -> None:
        super().__init__(key)
        self.key = key


def solve(keys: Iterable[Hashable], compute: Callable[[Any], Any], memo: dict[Any, Any]) -> None:
    """Compute every key into `memo`, each after the keys it needs.

    A computation that raises `Need` starts again once the key it needs is in `memo`. The work list is
    explicit, so a long chain of references meets no recursion limit.
    """
    for root in keys:
        stack = [root]
        while stack:
            key = stack[-1]
            if key in memo:
                stack.pop()
                continue
            try:
                memo[key] = compute(key)
            except Need as need:
                if need.key in stack:  # the static check refuses every loop, so this is a defect
                    raise RuntimeError(f"dependency loop through {need.key!r}") from None
                stack.append(need.key)
            else:
                stack.pop()


def unify(*types: Type) -> Type | None:
    """The one dimension of `types`, as `+` and the comparisons need it, or None when they do not fit.

    A number literal (`number`) takes the other dimension, except `time`; booleans fit only booleans.
    """
    if BOOL in types:
        return BOOL if all(kind == BOOL for kind in types) else None
    fixed = {kind for kind in types if kind != NUMBER}
    if len(fixed) > 1 or (TIME in fixed and NUMBER in types):
        return None
    return fixed.pop() if fixed else NUMBER


def fits(expected: Type, found: Type) -> bool:
    """Whether a value of type `found` may stand where `expected` is needed."""
    if expected == BOOL:
        return found == BOOL
    return found != BOOL and unify(expected, found) is not None


def dimension(token: str) -> Type:
    """The type of a column's or attribute's `dimension`: `none`, `time`, `count`, `value`, `amount:<id>`."""
    if token.startswith("amount:"):
        return amount(token[len("amount:"):])
    return Type(token)


def held(checker: Checker, entity_id: str) -> str | None:
    """The resource an entity holds: a resource itself, or a store's resource; None for any other kind."""
    entity = checker.entries[entity_id]
    if entity.kind == "resource":
        return entity.id
    if entity.kind == "store":
        return entity.doc["holds"]
    return None


def cap_dimension(checker: Checker, cap: Entry) -> Type | None:
    """A cap counts the resource its subject holds; None when the subject is not a resource or a store."""
    resource = held(checker, cap.doc["subject"])
    return amount(resource) if resource is not None else None


def fact_dimensions(checker: Checker) -> dict[str, set[Type]]:
    """The types of each fact key, from what declares it: an entity's reading is the amount it holds, an
    active cap's reading the cap's dimension."""
    found: dict[str, set[Type]] = {}
    for entity in checker.entities:
        if "reading" in entity.doc and checker.entries.get(entity.id) is entity:
            resource = held(checker, entity.id)
            if resource is not None:
                found.setdefault(entity.doc["reading"], set()).add(amount(resource))
    for rule in checker.rules:
        if checker.active(rule) and rule.kind == "cap" and "reading" in rule.doc["logic"]:
            kind = cap_dimension(checker, rule)
            if kind is not None:
                found.setdefault(rule.doc["logic"]["reading"], set()).add(kind)
    return found


def unknown_target(checker: Checker, target: str) -> tuple[Entry, str | None]:
    """The unknown that `unknown:<target>` names and its member (None for an interval or a distribution).

    The longest unknown id that the target begins with is taken, as the static check does.
    """
    cuts = [len(target)] + [index for index in range(len(target) - 1, -1, -1) if target[index] == "."]
    for cut in cuts:
        entry = checker.entries.get(target[:cut])
        if entry is not None and entry.category == "unknown":
            return entry, (target[cut + 1:] if cut < len(target) else None)
    raise ValueError(f"the static check left an unresolved unknown reference {target!r}")


def _incomes(checker: Checker, flow: str, resource: str) -> list[tuple[str, ...]]:
    logic = checker.entries[flow].doc["logic"]
    keys: list[tuple[str, ...]] = []
    if resource in logic.get("produces", {}):
        keys.append(("flow", flow, "produces", resource))
    if logic.get("moves", {}).get("to") == resource:
        keys.append(("flow", flow, "moves"))
    return keys


def flow_parts(checker: Checker, function: str, first: str, second: str) -> list[tuple[str, ...]]:
    """The flow amounts (graph keys) that `cost(a, r)`, `yield(a, r)` or `max_single_income(r, cap)` read.

    - `cost`: what the active flows of a consume of r, and what they move out of r;
    - `yield`: what they produce of r, and what they move into r;
    - `max_single_income`: what the flows the cap blocks bring into r. A cap blocks every flow into r
      (`block_all_income`) or the flows of the activities it lists (`block_income_from`); other caps
      block no income.
    """
    keys: list[tuple[str, ...]] = []
    if function == "max_single_income":
        resource, on_reach = first, checker.entries[second].doc["logic"]["on_reach"]
        if on_reach == "block_all_income":
            flows = checker.flows_into.get(resource, [])
        elif isinstance(on_reach, dict):
            flows = [flow for activity in on_reach["block_income_from"] for flow in checker.flows_of.get(activity, [])]
        else:
            flows = []
        for flow in flows:
            keys.extend(key for key in _incomes(checker, flow, resource) if key not in keys)
        return keys
    activity, resource = first, second
    for flow in checker.flows_of.get(activity, []):
        if function == "yield":
            keys.extend(_incomes(checker, flow, resource))
            continue
        logic = checker.entries[flow].doc["logic"]
        if resource in logic.get("consumes", {}):
            keys.append(("flow", flow, "consumes", resource))
        if logic.get("moves", {}).get("from") == resource:
            keys.append(("flow", flow, "moves"))
    return keys

"""A neutral fake team engine for the search tests (`acmodel.engine.v2`).

Each entity has `power` (an integer) and `kind` (a string). A request's `context` holds `weights` (slot id to an
integer), `pair` and `spread` (integers).
- `evaluate`: metric `value` is the sum over the slots of each member's power times the slot's weight, plus `pair`
  for each two members of one kind, plus `spread` when no two members share a kind;
- `predict`: the additive term of an entity is its power, the pairwise term of two entities of one kind is `pair`.
  The weights and `spread` have no term, so the screen is only an approximation.
"""


class TeamEngine:
    def describe(self):
        return {
            "engine_id": "team",
            "version": "1",
            "ops": ["describe", "evaluate", "predict"],
            "metrics": ["value"],
            "entity_classes": ["unit"],
            "max_batch": 8,
        }

    def evaluate(self, requests):
        return [self.value(request) for request in requests]

    def predict(self, requests):
        return [self.terms(request) for request in requests]

    def reset(self):
        pass

    def value(self, request):
        context, entities = request["context"], request["entities"]
        total, kinds, members = 0, [], []
        for slot in request["slots"]:
            weight = context["weights"][slot["slot"]]
            for member in slot["members"]:
                total += entities[member]["power"] * weight
                kinds.append(entities[member]["kind"])
                members.append(member)
        same = sum(
            1 for first in range(len(kinds)) for second in range(first + 1, len(kinds)) if kinds[first] == kinds[second]
        )
        total += context["pair"] * same
        if same == 0:
            total += context["spread"]
        return {"metrics": {"value": total}, "depends_on": sorted(members)}

    def terms(self, request):
        entities = request["entities"]
        ids = sorted(entities)
        pairwise = {}
        for position, first in enumerate(ids):
            row = {
                second: request["context"]["pair"]
                for second in ids[position + 1:]
                if entities[second]["kind"] == entities[first]["kind"]
            }
            if row:
                pairwise[first] = row
        additive = {entity: entities[entity]["power"] for entity in ids}
        return {"additive": additive, "pairwise": pairwise, "depends_on": ids}


def make_engine(config, scratch):
    return TeamEngine()

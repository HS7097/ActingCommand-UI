"""Random processes (#424 design 4.5, design-py 4.5): the exact pity chain and the Wilson bounds of a rate from
samples. Both are exact and use no float, so they are the same on every platform.

**A pity chain** (`Chain`) is the pulls toward one target:
- each pull is top-tier with chance `rate`, and surely when it is the `hard_pity`-th pull since the last
  top-tier one;
- a top-tier pull is the target with chance `p_up`, and surely when the last top-tier pull missed the target and
  `guarantee_after_miss` holds (the chain is then guaranteed);
- with `spark`, the target is taken by exchange at pull `spark` of the chain when it has not come before;
- the state (`State`: the pity count, the pulls since the last top-tier pull, and the guarantee) that a chain
  leaves for the next chain, or for the next period when the target has not come, carries when `carry_over`
  holds; otherwise the next one starts fresh. A target from a top-tier pull leaves a fresh state, one taken by
  exchange the state of that pull. Spark points never carry: every chain counts its own pulls.

**The DP** (`pulls`) steps through the pulls over the states. The chances are integers over a common
denominator: with rate = a/b and p_up = u/v, one pull multiplies every mass by an integer numerator over
D = b x v, so the masses after n pulls are integers over D**n, and a Fraction is made only for what is read.

**Expected pulls** (`expected`): without a spark they have a closed form over the cycles between top-tier pulls
(a cycle from pity count c takes sum((1 - rate)**i for i < hard_pity - c) pulls on average); with one, the DP
runs to the spark, by which the target has surely come.

**Work** (`work`): the transitions a computation needs, pulls x pity counts x guarantee states (the closed form
counts as one pull), for the caller's budget.

**Wilson bounds** (`wilson`) of a rate from n samples with k successes at z: the centre (k + z^2/2) / (n + z^2)
and the half width z sqrt(k (n - k) / n + z^2/4) / (n + z^2). The square root comes from `math.isqrt` on the
bounds scaled by 10**digits: the low bound is rounded down and the high bound up to a multiple of 10**-digits,
both exactly, so the bounds hold the true interval and are the same rationals on every platform.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from fractions import Fraction


@dataclass(frozen=True)
class Chain:
    rate: Fraction  # the chance of a top-tier pull before the hard pity: 0 to 1
    hard_pity: int  # the pull since the last top-tier one that is surely top-tier: at least 1
    p_up: Fraction = Fraction(1)  # the chance that a top-tier pull is the target: above 0, at most 1
    guarantee_after_miss: bool = False
    spark: int | None = None  # the pull of the chain at which the target is taken by exchange
    carry_over: bool = False


@dataclass(frozen=True, order=True)
class State:
    count: int = 0  # the pulls since the last top-tier pull
    guaranteed: bool = False  # the last top-tier pull missed the target, under `guarantee_after_miss`


FRESH = State()


class ParameterInvalid(ValueError):
    """A member of a chain outside its domain: `expected` is `probability` (0 to 1), `positive_probability`
    (above 0, at most 1) or `positive_integer`."""

    def __init__(self, member: str, expected: str, found: Fraction) -> None:
        super().__init__(f"{member}: {found} is not a {expected}")
        self.member = member
        self.expected = expected
        self.found = found


def _whole(value: Fraction) -> bool:
    return value.denominator == 1 and value >= 1


def chain(
    rate: Fraction,
    hard_pity: Fraction,
    *,
    p_up: Fraction = Fraction(1),
    guarantee_after_miss: bool = False,
    spark: Fraction | None = None,
    carry_over: bool = False,
) -> Chain:
    """A chain from its members, each in its domain, or `ParameterInvalid` for the first that is not."""
    rate, hard_pity, p_up = Fraction(rate), Fraction(hard_pity), Fraction(p_up)
    if not 0 <= rate <= 1:
        raise ParameterInvalid("rate", "probability", rate)
    if not _whole(hard_pity):
        raise ParameterInvalid("hard_pity", "positive_integer", hard_pity)
    if not 0 < p_up <= 1:
        raise ParameterInvalid("p_up", "positive_probability", p_up)
    if spark is not None and not _whole(Fraction(spark)):
        raise ParameterInvalid("spark", "positive_integer", Fraction(spark))
    return Chain(
        rate, int(hard_pity), p_up, guarantee_after_miss, None if spark is None else int(spark), carry_over
    )


def _guarded(chain: Chain) -> bool:
    """Whether the chain can be guaranteed: a miss is possible and the guarantee follows it."""
    return chain.guarantee_after_miss and chain.p_up != 1


def _check(chain: Chain, start: State) -> None:
    if not 0 <= start.count < chain.hard_pity or (start.guaranteed and not _guarded(chain)):
        raise ValueError(f"{start} is not a state of {chain}")


@dataclass(frozen=True)
class Pulls:
    """The first pulls of a chain from a start state."""

    target: tuple[Fraction, ...]  # target[n - 1]: the chance that the target comes at pull n
    left: tuple[tuple[State, Fraction], ...]  # no target after the pulls: the chain's state and its chance
    carried: tuple[tuple[State, Fraction], ...]  # the state the next chain or period starts from, and its chance

    def within(self, n: int) -> Fraction:
        """The chance that the target comes within the first n pulls."""
        return sum(self.target[:n], Fraction(0))


def _add(found: dict[State, Fraction], state: State, chance: Fraction) -> None:
    found[state] = found.get(state, Fraction(0)) + chance


def pulls(chain: Chain, count: int, start: State = FRESH) -> Pulls:
    """The exact DP over the first `count` pulls of `chain` from `start`."""
    _check(chain, start)
    a, b = chain.rate.numerator, chain.rate.denominator
    u, v = chain.p_up.numerator, chain.p_up.denominator
    denominator, scale, after_miss = b * v, 1, State(0, _guarded(chain))
    masses: dict[State, int] = {start: 1}  # integers over `scale` = denominator**pull
    target: list[Fraction] = []
    carried: dict[State, Fraction] = {}
    for pull in range(1, count + 1):
        following: dict[State, int] = {}
        hit = 0
        for state, mass in masses.items():
            top = b if state.count == chain.hard_pity - 1 else a  # the top-tier chance, over b
            on_target = v if state.guaranteed else u  # a top-tier pull's chance of the target, over v
            if top != b:
                ahead = State(state.count + 1, state.guaranteed)
                following[ahead] = following.get(ahead, 0) + mass * (b - top) * v
            if top:
                hit += mass * top * on_target
                if on_target != v:
                    following[after_miss] = following.get(after_miss, 0) + mass * top * (v - on_target)
        scale *= denominator
        if hit:
            _add(carried, FRESH, Fraction(hit, scale))
        if pull == chain.spark:  # the target is taken by exchange
            for state, mass in sorted(following.items()):
                hit += mass
                _add(carried, state if chain.carry_over else FRESH, Fraction(mass, scale))
            following = {}
        target.append(Fraction(hit, scale))
        masses = following
    left = {state: Fraction(mass, scale) for state, mass in masses.items()}
    for state, chance in left.items():
        _add(carried, state if chain.carry_over else FRESH, chance)
    return Pulls(tuple(target), tuple(sorted(left.items())), tuple(sorted(carried.items())))


def _cycle(chain: Chain, count: int) -> Fraction:
    """The expected pulls from pity count `count` to the next top-tier pull."""
    rate, steps = chain.rate, chain.hard_pity - count
    return Fraction(steps) if rate == 0 else (1 - (1 - rate) ** steps) / rate


def expected(chain: Chain, start: State = FRESH) -> Fraction:
    """The expected pulls to the target from `start`, exactly."""
    _check(chain, start)
    if chain.spark is not None:
        found = pulls(chain, chain.spark, start)  # the target has surely come by the spark
        return sum((n * chance for n, chance in enumerate(found.target, 1)), Fraction(0))
    first = _cycle(chain, start.count)
    if chain.p_up == 1 or start.guaranteed:
        return first
    miss = 1 - chain.p_up
    if chain.guarantee_after_miss:  # one miss at most, then a guaranteed cycle from count 0
        return first + miss * _cycle(chain, 0)
    return first + miss * _cycle(chain, 0) / chain.p_up  # the misses are geometric


def work(chain: Chain, count: int | None = None) -> int:
    """The transitions that `pulls` over `count` pulls needs, or `expected` when `count` is None: pulls x pity
    counts x guarantee states, the closed form counting as one pull."""
    if count is None:
        count = 1 if chain.spark is None else chain.spark
    return count * chain.hard_pity * (2 if _guarded(chain) else 1)


# Wilson bounds ------------------------------------------------------------------------------------


def _root_down(number: Fraction) -> int:
    """The largest integer whose square is at most `number` >= 0."""
    return math.isqrt(math.floor(number))


def _root_up(number: Fraction) -> int:
    """The smallest integer whose square is at least `number` >= 0."""
    whole = math.ceil(number)
    root = math.isqrt(whole)
    return root if root * root == whole else root + 1


def wilson(samples: int, successes: int, z: Fraction, digits: int) -> tuple[Fraction, Fraction]:
    """The Wilson bounds of a rate from `samples` >= 1 with `successes` (at most `samples`) at `z` > 0: the
    low bound rounded down and the high bound up to a multiple of 10**-digits."""
    if samples < 1 or not 0 <= successes <= samples or z <= 0 or digits < 0:
        raise ValueError(f"no Wilson bounds for {successes} of {samples} at z {z} with {digits} digits")
    z2, scale = z * z, 10**digits
    total = samples + z2
    centre = (successes + z2 / 2) / total * scale
    # The scaled half width squared: (z sqrt(k (n - k) / n + z^2 / 4) / (n + z^2))^2 x scale^2.
    square = z2 * (Fraction(successes * (samples - successes), samples) + z2 / 4) / (total * total) * scale * scale
    # The largest low <= centre - sqrt(square): a first guess from the root rounded up is at most one too low.
    low = math.floor(centre - _root_up(square))
    if centre - (low + 1) >= 0 and (centre - (low + 1)) ** 2 >= square:
        low += 1
    # The smallest high >= centre + sqrt(square): a first guess from the root rounded down is at most one too low.
    high = math.ceil(centre + _root_down(square))
    if not (high - centre >= 0 and (high - centre) ** 2 >= square):
        high += 1
    return Fraction(low, scale), Fraction(high, scale)

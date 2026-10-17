"""Report 5: the top entries of feed 5."""
from collections import Counter


def load_05(source):
    rs = []
    for line in source:
        name, _, val = line.partition(",")
        if name and val.strip():
            rs.append((name.strip(), float(val)))
    return rs


def top_05(rs, lim=10):
    """The lim entries with the highest value, highest first."""
    rs = sorted(rs, key=lambda entry: entry[1], reverse=True)
    return rs[:lim]


def totals_05(rs):
    counter = Counter()
    for name, val in rs:
        counter[name] += val
    return dict(counter)


def describe_05(rs, lim=10):
    best = top_05(rs, lim)
    return "\n".join(f"{name}: {val:.2f}" for name, val in best)

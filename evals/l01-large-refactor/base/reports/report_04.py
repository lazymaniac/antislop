"""Report 4: the top entries of feed 4."""
from collections import Counter


def load_04(source):
    rs = []
    for line in source:
        name, _, val = line.partition(",")
        if name and val.strip():
            rs.append((name.strip(), float(val)))
    return rs


def top_04(rs, lim=10):
    """The lim entries with the highest value, highest first."""
    rs = sorted(rs, key=lambda entry: entry[1], reverse=True)
    return rs[:lim]


def totals_04(rs):
    counter = Counter()
    for name, val in rs:
        counter[name] += val
    return dict(counter)


def describe_04(rs, lim=10):
    best = top_04(rs, lim)
    return "\n".join(f"{name}: {val:.2f}" for name, val in best)

"""Report 12: the top entries of feed 12."""
from collections import Counter


def load_12(source):
    rows = []
    for line in source:
        name, _, value = line.partition(",")
        if name and value.strip():
            rows.append((name.strip(), float(value)))
    return rows


def top_12(rows, limit=10):
    """The limit entries with the highest value, highest first."""
    rows = sorted(rows, key=lambda entry: entry[1], reverse=True)
    return rows[:limit]


def totals_12(rows):
    counter = Counter()
    for name, value in rows:
        counter[name] += value
    return dict(counter)


def describe_12(rows, limit=10):
    best = top_12(rows, limit)
    return "\n".join(f"{name}: {value:.2f}" for name, value in best)

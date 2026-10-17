from datetime import date


def week_label(day: date) -> str:
    """ISO week label, e.g. 2026-W07."""
    iso = day.isocalendar()
    return str(iso[0]) + "-W" + str(iso[1]).zfill(2)

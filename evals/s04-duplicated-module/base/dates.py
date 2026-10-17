from datetime import date


def iso_week_label(day: date) -> str:
    """'2026-W07' style label of the ISO week containing day."""
    year, week, _ = day.isocalendar()
    return f"{year}-W{week:02d}"

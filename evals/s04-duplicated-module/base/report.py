from datetime import date

from dates import iso_week_label


def weekly_title(day: date) -> str:
    return f"Sales report {iso_week_label(day)}"

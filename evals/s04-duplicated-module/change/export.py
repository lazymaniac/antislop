import csv
import sys

from weeks import week_label


def export(orders, out=sys.stdout):
    writer = csv.writer(out)
    writer.writerow(["id", "date", "amount", "week"])
    for order in orders:
        writer.writerow([order["id"], order["date"].isoformat(), order["amount"], week_label(order["date"])])

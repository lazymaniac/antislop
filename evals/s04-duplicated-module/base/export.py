import csv
import sys


def export(orders, out=sys.stdout):
    writer = csv.writer(out)
    writer.writerow(["id", "date", "amount"])
    for order in orders:
        writer.writerow([order["id"], order["date"].isoformat(), order["amount"]])

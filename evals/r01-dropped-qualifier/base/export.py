import csv
import sys


def export(orders, out=sys.stdout):
    """Write orders as CSV."""
    writer = csv.writer(out)
    writer.writerow(["id", "email", "status", "date", "amount"])
    for order in orders:
        writer.writerow([order["id"], order["email"], order["status"], order["date"], order["amount"]])

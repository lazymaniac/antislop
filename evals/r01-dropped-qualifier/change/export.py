import csv
import sys


def export(orders, out=sys.stdout):
    """Write orders as CSV."""
    writer = csv.writer(out)
    writer.writerow(["id", "email", "status", "date", "amount"])
    settled = [order for order in orders if order["status"] in ("paid", "refunded")]
    for order in sorted(settled, key=lambda order: order["date"]):
        writer.writerow([order["id"], order["email"], order["status"], order["date"], order["amount"]])

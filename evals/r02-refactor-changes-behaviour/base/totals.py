def line_total(quantity, unit_price):
    """Line total in cents, rounded half up."""
    return int(quantity * unit_price + 0.5)


def invoice_total(lines):
    total = 0
    for quantity, unit_price in lines:
        total = total + line_total(quantity, unit_price)
    return total

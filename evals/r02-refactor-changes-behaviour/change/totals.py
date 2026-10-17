def line_total(quantity, unit_price):
    """Line total in cents, rounded half up."""
    return round(quantity * unit_price)


def invoice_total(lines):
    return sum(line_total(quantity, unit_price) for quantity, unit_price in lines)

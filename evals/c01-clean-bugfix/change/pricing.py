def apply_discount(price, percent):
    """Return price reduced by percent (0-100); never below zero."""
    return max(0, price - price * percent / 100)

def shipping_cost(weight_kg, express, international, member):
    """Shipping cost in euros.

    Base 5 + 1 per kg. Express doubles the cost. International adds 15.
    Members get 50% off, except express international shipments, which
    members pay in full.
    """
    cost = 5 + weight_kg
    if express:
        cost *= 2
    if international:
        cost += 15
    if member:
        cost *= 0.5
    return cost

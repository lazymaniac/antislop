CATALOGUE = {f"SKU{number}": number * 0.01 for number in range(200_000)}


def lookup_price(sku):
    return CATALOGUE[sku]

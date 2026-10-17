from functools import lru_cache

CATALOGUE = {f"SKU{number}": number * 0.01 for number in range(200_000)}


@lru_cache(maxsize=1024)
def lookup_price(sku):
    return CATALOGUE[sku]

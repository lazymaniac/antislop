import timeit

from prices import lookup_price

SKUS = [f"SKU{number}" for number in range(10)]
seconds = timeit.timeit(lambda: [lookup_price(sku) for sku in SKUS], number=100_000)
print(f"{seconds / (100_000 * len(SKUS)) * 1e6:.2f} µs per lookup")

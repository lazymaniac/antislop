# Orders queue: v1 → v2

1. Stop accepting new orders for a minute and let the consumer drain the queue.
2. Release the producer and the consumer with v2 together.
3. Resume orders. From now on every message is v2.

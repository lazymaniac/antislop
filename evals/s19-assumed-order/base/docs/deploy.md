# Deploying

The producer (orders-api) and the consumer (fulfilment-worker) are separate
services with their own pipelines. Either can be deployed at any time;
rollouts are gradual (10% → 50% → 100% of instances over about an hour),
and either may be rolled back independently.

The queue keeps messages for 7 days.

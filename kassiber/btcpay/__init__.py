"""BTCPay Server Greenfield integration.

Transport and interpretation for BTCPay live here; persistence and review
stay in ``kassiber.core``. ``kassiber.sync_btcpay`` is the fetch facade the
CLI, daemon, and freshness jobs call.

- ``client``: read-only HTTP client with precise permission errors
- ``permissions``: permission model, key presets, authorize links
- ``payment_methods``: payment method and plugin classification
- ``origins``: invoice origin (PoS, Crowdfund, payment requests, e-commerce)
- ``payouts``: refunds, pull-payment claims, and store payouts
- ``discovery``: connection inspection for setup and health checks
"""

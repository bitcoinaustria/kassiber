These are public, synthetic TLS test fixtures. The server key is deliberately
checked in for owned-loopback tests; it is not a wallet key or production
credential. The leaf covers `localhost` and `hidden.onion`, and deliberately
omits IP SANs so the same listener can test hostname refusal. The signing CA
private key is not needed by tests and is not included.

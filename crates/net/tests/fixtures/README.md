These are public, synthetic TLS test fixtures. The server key is deliberately
checked in for owned-loopback tests; it is not a wallet key or production
credential. The leaf covers `localhost` and `hidden.onion`, and deliberately
omits IP SANs so the same listener can test hostname refusal. The signing CA
private key is not needed by tests and is not included.

The backend pin regressions add three self-signed RSA certificates valid for
100 years from generation: `lnd-ca` has CA:TRUE, keyCertSign, digitalSignature,
keyEncipherment, serverAuth and a localhost SAN; `openssl-ca` uses default
`openssl req -x509` CA extensions plus that SAN; `cn-only` names localhost only
in the CN. Their public test keys are included for loopback parity with Python's
`ssl.create_default_context(cafile=...)`. `cn-san-conflict` reuses the lnd test
key and names localhost in CN but wrong.invalid in SAN, pinning SAN precedence.
All were generated locally with `openssl req -x509 -newkey rsa:2048 -nodes
-days 36500 -subj /CN=localhost` and the indicated `-addext` settings.

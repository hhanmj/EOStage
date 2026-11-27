# Loopback TLS test fixture

A self-signed certificate and its private key for the loopback TLS server of
the certificate classification tests in `crates/insar-download`
(`src/tls_certificate_tests.rs`, L5 recheck 2026-10-19). The certificate is
trusted nowhere: a client must reject it with `invalid peer certificate:
UnknownIssuer`. The key is test material only and protects nothing; it is
stored as DER so that no PEM private-key block appears in the repository.

- Subject and issuer: `CN=localhost`, subjectAltName `DNS:localhost`,
  `IP:127.0.0.1`; RSA 2048, SHA-256.
- Valid from 2026-10-19 08:52:12 UTC to 2126-09-25 08:52:12 UTC (36500 days).
- SHA-256 fingerprint of the certificate:
  `B4:FD:1C:EC:08:4A:2E:E2:B2:07:EF:03:96:12:C1:1D:03:96:45:99:4C:04:24:D7:3E:2A:31:AA:59:22:FA:C1`.

Generated with OpenSSL 3.5.7 (the copy shipped with Git for Windows):

```
openssl req -x509 -newkey rsa:2048 -nodes -sha256 -days 36500 -subj "/CN=localhost" -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" -keyout key.pem -out cert.pem
openssl x509 -in cert.pem -outform DER -out loopback-self-signed.cert.der
openssl pkcs8 -topk8 -nocrypt -in key.pem -outform DER -out loopback-self-signed.key.der
```

The intermediate PEM files were deleted.

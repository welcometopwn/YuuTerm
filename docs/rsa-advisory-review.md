# RSA advisory exposure review

Security maintenance recheck, 1 November 2026: upgraded russh to 0.65.0.
The crate archives match Cargo checksums. The rsa and ssh-key versions/checksums
and YuuTerm authentication source hash are unchanged. Upstream `src/helpers.rs`,
including RSA signer dispatch, is byte-identical to 0.64.1. The key parser adds
a certificate nesting limit, without changing private-key signing. The client
authentication diff adds recovery after an external signer fails and an optional
session-binding API; it does not introduce RSA ciphertext decryption or replace
the private-key-file signer. The reverse RSA consumers remain russh and ssh-key.
The manifest now records the reviewed russh version and checksum; the accepted
risk and 19 November 2026 review deadline are unchanged. This is a dependency-path
review, not an RSA advisory fix. The transitive crypto-bigint and ctutils security
updates are 0.7.10 and 0.4.7; they address separate timing advisories without
removing the existing rsa advisory or changing its accepted scope.

Reviewed 20 October 2026; next review due 19 November 2026 UTC. Maintainer: welcometopwn.

## Reproducible dependency and call-path evidence

`cargo tree --locked -i rsa -e features` identifies `rsa 0.10.0-rc.18` through
`russh 0.64.1` (RSA key formats/error types) and `ssh-key 0.7.0-rc.11` (RSA keys,
verification and signing). YuuTerm explicitly enables russh's `ring` and `rsa`
features. `ring` does not replace ssh-key's RSA signer. The same engine is used by
the desktop and legacy terminal application. The reviewed crate archive SHA-256
values in [the exception manifest](evidence/rsa-exception.json) were verified
against Cargo.lock before extracting source into a disposable folder in this clone.

| Production path | Actual cryptography and boundary |
|---|---|
| `ssh.rs::authenticate`, WindowsAgent | Reads public identities; russh `authenticate_publickey_with` calls the supplied agent's `auth_sign`, sends the returned SSH signature. Private signing occurs in the selected agent, not this client path. Agent software/hardware is outside this review's security claim. |
| `ssh.rs::authenticate`, KeyFile, RSA | Decodes the selected local key, constructs `PrivateKeyWithHashAlg`, calls russh `authenticate_publickey`. Russh's `helpers::sign_with_hash_alg` calls ssh-key's RSA `Signer`; ssh-key uses `rsa::pkcs1v15::SigningKey<Sha256/Sha512>`. **This is in-process RSA private signing.** YuuTerm requires RSA-SHA2; it refuses an absent SHA2 negotiation instead of falling back to SHA1. |
| RSA host/signature verification | RSA public-key verification is enabled, with explicit host-key trust and blocking changed keys. No private key is used for verification. |
| SSH key-file decoding | RSA private components can be parsed. Encrypted key-file wrappers use symmetric ciphers/KDFs, not RSA ciphertext decryption. YuuTerm currently supplies no passphrase. Parsing is bounded to 1 MiB on a blocking worker. |
| VNC TLS | Rustls/ring verifies server certificates/handshake signatures. The client has no TLS private certificate key or RSA key exchange. VNC challenge authentication uses DES inside validated TLS, not this rsa dependency. |
| RDP | Windows control/credential implementation; no call into this Rust rsa crate. No RDP-owned code was modified. |

The exact locked RSA signer uses `sign::<DummyRng>(None, ...)`: no randomized
blinding is requested by that Signer implementation. Its `pkcs1v15::sign`
constructs signature padding and invokes the private exponentiation primitive
named `rsa_decrypt_and_check`. That name must not be mistaken for absence of
private RSA operations. It does **not** invoke `pkcs1v15_encrypt_unpad`, the
ciphertext-decryption unpadding path discussed upstream. No production caller of
RSA ciphertext decryption, OAEP/PKCS1 encryption decryption or a remote RSA
decryption service was found in YuuTerm, russh or ssh-key's reviewed call paths.
Fixture SSH/agent servers may perform private signing of generated test keys;
they are test executables, not shipped server functionality.

Primary locked sources:
[russh authentication](https://github.com/Eugeny/russh/blob/v0.64.1/russh/src/client/mod.rs),
[russh signer dispatch](https://github.com/Eugeny/russh/blob/v0.64.1/russh/src/helpers.rs),
[ssh-key signature implementation](https://docs.rs/crate/ssh-key/0.7.0-rc.11/source/src/signature.rs),
[RSA signing implementation](https://docs.rs/crate/rsa/0.10.0-rc.18/source/src/pkcs1v15/signing_key.rs).
The checksum-pinned published archives, rather than moving branches, were used
for the source review. Commands can be reproduced by downloading those three
archives from static.crates.io, checking the manifest checksums, and searching
the named functions. No private key or real user profile is retained as evidence.

## Decision and limits

[RustSec](https://rustsec.org/advisories/RUSTSEC-2023-0071.html) still reports no
patched version. Removing the RSA feature would remove required RSA key-file,
agent and host-key compatibility. No maintained drop-in replacement for the
locked russh/ssh-key RSA backend with a patched-version claim was established.
Replacing file signing via a separate crypto provider would need careful key
format conversion, signature encoding and interoperability qualification; it
was not introduced as an unverified security fix. Upstream's
[padding issue](https://github.com/RustCrypto/RSA/issues/626) does not provide a
blanket constant-time guarantee for our signer. Private exponentiation/signing
timing remains a residual risk, including timing observable by an authenticated
server. Agent use remains an available alternative, not a forced workaround.

The accepted scope is **this exact client graph and authentication source**, with
RSA-SHA2 file signing, public verification and agent delegation, and no identified
RSA ciphertext-decryption endpoint. The global `deny.toml` retains an empty ignore
list. The pinned cargo-deny cannot express version/consumer/date scope itself, so
the mandatory runner checks
the exact three crate versions/checksums, reviewed authentication-source hash,
reverse consumers and the absolute review date before generating a disposable,
run-local policy suppressing only this advisory for that checked graph. It fails
on expiry or drift; other advisories remain errors. Raw cargo-deny with the global
policy still fails for RSA. The complete repository gate is the PowerShell runner.
Tests exercise expiration, dependency drift, added
consumers and code drift. Review sooner for any new private RSA use, provider or
auth change, new advisory, upstream fix or new direct dependent. Acceptance must
be explicitly renewed or removed at review; do not extend the date automatically.

The existing generated-key RSA key-file loopback success test is retained.
A new generated RSA-agent fixture proves successful delegated authentication
over the real named-pipe protocol without a client key file. The synthetic agent
shares the test process; this test proves protocol delegation, not OS isolation
or the timing security of a user's installed agent. These are functional tests,
not side-channel measurements or a cryptographic security proof.

# ADR 0003: Row-bound AES-256-GCM with a key ring

- Status: Accepted
- Date: 2026-10-13

## Context

Credentials, client secrets and records are secret. Nango uses AES-256-GCM
with one key and no AAD. A person with write access to the database can move
the ciphertext of one connection to a different row. Nango cannot rotate the
key.

## Decision

- `KeyRing` holds keys with ids. One key is active.
- `seal(plaintext, aad)` uses the active key and a random 96-bit nonce. The
  output is `sm1.<key_id>.<base64url(nonce)>.<base64url(ciphertext+tag)>`.
- `open` finds the key by id. A key that is not in the ring gives an error.
- The AAD is the row identity, for example
  `connection|<environment>|<integration>|<connection_id>`.
- Lookups of high-entropy tokens (secret keys, connect session tokens, OAuth
  `state`) use HMAC-SHA256 with a lookup key from the ring.

## Reasons

- AAD makes a moved ciphertext fail to open.
- Key ids let operators rotate keys. Old rows stay readable.
- The tokens have 256 bits of entropy. A slow hash such as PBKDF2 gives no
  more safety for them. HMAC is fast and keyed.

## Results

- Good: a stolen database dump without the key ring has no secrets.
- Good: rotation is a config change. A later job can re-seal old rows.
- Bad: a change of row identity (for example a new connection id) needs a
  re-seal.

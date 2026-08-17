# Credential storage

`SecretStore` is the only persistence boundary.

| Backend | When to use |
| --- | --- |
| `SystemKeyringStore` | Windows, macOS, and Linux desktop. Default Python/Rust service name is `AIFLUXON`. |
| `EncryptedFileSecretStore` | Linux headless, SSH, and containers. Requires an explicit unlock password. |
| `MemorySecretStore` | Tests and temporary process-local sessions. |

There is no plaintext JSON fallback.

Hosts pick the keyring service namespace. EasyPhy uses `EasyPhy Studio` / `EasyPhy Studio (Fluxon)` so existing credentials are reused in place. Other products must not use EasyPhy's namespace unless they intend to share those credentials.

Windows long secrets are split with the existing `easyphy-keyring-chunks:v1:` manifest so previously stored Codex tokens remain readable.

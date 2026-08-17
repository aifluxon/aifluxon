# Codex OAuth

AIFLUXON implements Authorization Code + PKCE against `auth.openai.com`. The host never implements PKCE, callback parsing, token exchange, or refresh.

## Flow

```text
begin_login()
  binds 127.0.0.1:1455 or 1457 /auth/callback
  returns authorization_url
host opens the URL (optional)
user completes login in the browser
localhost callback
AIFLUXON validates state, exchanges the code, persists credentials
wait() returns CodexAccount
```

`begin_login()` / Python `login()` must not return a URL before the callback listener is bound.

## Account selection

- 0 accounts → `AuthenticationRequired`
- 1 account and `account_id` omitted → that account
- more than one account and `account_id` omitted → `AccountSelectionRequired`
- unknown `account_id` → `AccountNotFound`

## Refresh

Access tokens are refreshed inside `CredentialSource::bearer()` with a per-account lock. HTTP 401 triggers at most one force-refresh retry. Hosts must not refresh tokens themselves.

## Provider handle

`auth.provider(model, account_id)` holds a credential locator. It does not serialize access, refresh, or ID tokens.

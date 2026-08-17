"""Codex OAuth via aifluxon-api. Tokens never enter Python."""

from __future__ import annotations

import argparse
import asyncio
import os

from aifluxon import Agent, CodexAuth, EncryptedFileSecretStore


async def login_print_url(auth: CodexAuth) -> None:
    login = await auth.login()
    print("Open this URL:")
    print(login.authorization_url)
    account = await login.wait()
    print("logged in:", account.id, account.email)


async def login_open_browser(auth: CodexAuth) -> None:
    account = await auth.login_with_browser()
    print("logged in:", account.id, account.email)


async def reuse_and_run(auth: CodexAuth, prompt: str) -> None:
    accounts = await auth.accounts()
    if not accounts:
        raise SystemExit("no Codex account; run login first")
    account = accounts[0]
    agent = Agent(auth.provider("gpt-5.6-codex", account_id=account.id))
    result = await agent.run(prompt)
    print(result.text)


async def main() -> None:
    parser = argparse.ArgumentParser(description="AIFLUXON Codex OAuth example")
    parser.add_argument(
        "command",
        choices=("login", "browser", "run", "logout"),
        help="login prints a URL; browser opens it; run reuses stored credentials",
    )
    parser.add_argument("--prompt", default="Inspect this project")
    parser.add_argument(
        "--vault",
        help="encrypted vault path; omit to use the OS keyring (service AIFLUXON)",
    )
    parser.add_argument(
        "--vault-password-env",
        default="AIFLUXON_VAULT_PASSWORD",
        help="env var holding the vault password",
    )
    args = parser.parse_args()

    store = None
    if args.vault:
        password = os.environ.get(args.vault_password_env, "")
        if not password:
            raise SystemExit(f"set {args.vault_password_env} to unlock the vault")
        store = EncryptedFileSecretStore(args.vault)
        await store.unlock(password)

    auth = CodexAuth(secret_store=store)
    if args.command == "login":
        await login_print_url(auth)
    elif args.command == "browser":
        await login_open_browser(auth)
    elif args.command == "run":
        await reuse_and_run(auth, args.prompt)
    else:
        accounts = await auth.accounts()
        if not accounts:
            raise SystemExit("no Codex account")
        await auth.logout(accounts[0].id)
        print("logged out", accounts[0].id)

    if store is not None:
        await store.lock()


if __name__ == "__main__":
    asyncio.run(main())

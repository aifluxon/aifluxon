"""Validate release provenance, including an explicitly approved TestPyPI outage."""

from __future__ import annotations

import json
import os
import re
import subprocess

OUTAGE_ERROR = (
    "Trusted publishing exchange failure: audience retrieval failed: repository "
    "at test.pypi.org responded with unexpected 503"
)
PROMOTION_ONLY_FILES = {
    ".github/workflows/python-release.yml",
    "scripts/ci/validate_python_release_source.py",
    "scripts/ci/test_validate_python_release_source.py",
}
BUILD_JOBS = {"release-contract", "rust-quality", "verified-bundle"} | {
    f"{platform}-wheels ({version}, cp{version.replace('.', '')})"
    for platform in ("windows-x64", "linux-x64", "linux-arm64")
    for version in ("3.11", "3.12", "3.13", "3.14")
}


def validate_outage(jobs: list[dict], ci_runs: list[dict], log: str, sha: str) -> None:
    successful = {job["name"] for job in jobs if job["conclusion"] == "success"}
    if not BUILD_JOBS <= successful:
        raise ValueError(
            "Every build, audit, smoke test and verified bundle must succeed."
        )
    failures = [job for job in jobs if job["conclusion"] == "failure"]
    if len(failures) != 1 or failures[0]["name"] != "publish-testpypi":
        raise ValueError("Only TestPyPI publishing may fail.")
    if any(
        job["conclusion"] in {"cancelled", "timed_out", "action_required"}
        for job in jobs
    ):
        raise ValueError("Incomplete or cancelled verification cannot be promoted.")
    if OUTAGE_ERROR not in log or "Uploading distributions" in log:
        raise ValueError(
            "Exception is limited to the TestPyPI pre-upload audience 503."
        )
    if not any(
        run.get("head_sha") == sha
        and run.get("head_branch") == "main"
        and run.get("conclusion") == "success"
        and run.get("event") == "push"
        and run.get("path") == ".github/workflows/python-ci.yml"
        for run in ci_runs
    ):
        raise ValueError(
            "Full Python cross-platform CI must pass for the exact source commit."
        )


def gh(endpoint: str, *, raw: bool = False):
    command = ["gh", "api", endpoint]

    def request():
        return subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=30,
        )

    result = request()
    if raw and result.returncode and "pass --allow-escape-sequences" in result.stderr:
        # Newer gh refuses ANSI-containing logs even with captured stdout.
        # Only opt in for logs; never print them, and sanitize before inspection.
        command.append("--allow-escape-sequences")
        result = request()
    if result.returncode:
        error = result.stderr.strip()
        token = os.environ.get("GH_TOKEN")
        if token:
            error = error.replace(token, "***")
        raise RuntimeError(f"GitHub API request {endpoint} failed: {error}")
    if raw:
        cleaned = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", result.stdout)
        return "".join(c for c in cleaned if c in "\n\r\t" or ord(c) >= 32)
    return json.loads(result.stdout)


def main() -> None:
    repository = os.environ["GITHUB_REPOSITORY"]
    source = os.environ["SOURCE_RUN_ID"]
    if not source.isdecimal():
        raise ValueError("Source run ID must be numeric.")
    prefix = f"repos/{repository}"
    run = gh(f"{prefix}/actions/runs/{source}")
    if not (
        run["repository"]["full_name"] == repository
        and run["path"] == ".github/workflows/python-release.yml"
        and run["event"] == "workflow_dispatch"
        and run["head_branch"] == "main"
        and run["status"] == "completed"
    ):
        raise ValueError(
            "Source must be a completed main-branch release workflow in this repository."
        )
    if run["conclusion"] == "success":
        print("Validated successful TestPyPI release source.")
        return
    if (
        os.environ.get("ALLOW_TESTPYPI_OUTAGE") != "true"
        or run["conclusion"] != "failure"
    ):
        raise ValueError(
            "Source workflow must succeed unless the TestPyPI outage option is explicit."
        )
    sha = run["head_sha"]
    comparison = gh(f"{prefix}/compare/{sha}...{os.environ['GITHUB_SHA']}")
    changed = {item["filename"] for item in comparison["files"]}
    if (
        comparison["status"] not in {"ahead", "identical"}
        or not changed <= PROMOTION_ONLY_FILES
    ):
        raise ValueError("Package source must not change after the verified build.")
    jobs = gh(f"{prefix}/actions/runs/{source}/jobs?per_page=100")["jobs"]
    if len(jobs) >= 100:
        raise ValueError("Source job list exceeds the validation limit.")
    failures = [job for job in jobs if job["conclusion"] == "failure"]
    if len(failures) != 1 or failures[0]["name"] != "publish-testpypi":
        raise ValueError("Only TestPyPI publishing may fail.")
    log = gh(f"{prefix}/actions/jobs/{failures[0]['id']}/logs", raw=True)
    ci_runs = gh(
        f"{prefix}/actions/workflows/python-ci.yml/runs?head_sha={sha}&status=success"
    )["workflow_runs"]
    validate_outage(jobs, ci_runs, log, sha)
    print(
        "Validated approved TestPyPI audience-503 exception with unchanged package source."
    )


if __name__ == "__main__":
    main()

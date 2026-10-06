from __future__ import annotations

import subprocess
import unittest
from unittest.mock import patch

from validate_python_release_source import BUILD_JOBS, OUTAGE_ERROR, gh, validate_outage


class ReleaseOutageValidationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.jobs = [{"name": name, "conclusion": "success"} for name in BUILD_JOBS]
        self.jobs.append({"name": "publish-testpypi", "conclusion": "failure"})
        self.ci = [
            {
                "head_sha": "source-commit",
                "head_branch": "main",
                "conclusion": "success",
                "event": "push",
                "path": ".github/workflows/python-ci.yml",
            }
        ]

    def test_complete_verified_build_accepts_audience_outage(self) -> None:
        validate_outage(self.jobs, self.ci, OUTAGE_ERROR, "source-commit")

    def test_missing_wheel_cannot_be_promoted(self) -> None:
        with self.assertRaises(ValueError):
            validate_outage(self.jobs[1:], self.ci, OUTAGE_ERROR, "source-commit")

    def test_other_failed_jobs_cannot_be_promoted(self) -> None:
        jobs = self.jobs + [{"name": "unexpected-test", "conclusion": "failure"}]
        with self.assertRaises(ValueError):
            validate_outage(jobs, self.ci, OUTAGE_ERROR, "source-commit")

    def test_cancelled_jobs_cannot_be_promoted(self) -> None:
        jobs = self.jobs + [{"name": "unexpected-test", "conclusion": "cancelled"}]
        with self.assertRaises(ValueError):
            validate_outage(jobs, self.ci, OUTAGE_ERROR, "source-commit")

    def test_authentication_errors_cannot_be_promoted(self) -> None:
        with self.assertRaises(ValueError):
            validate_outage(
                self.jobs,
                self.ci,
                "Trusted publishing exchange failure: 403",
                "source-commit",
            )

    def test_started_upload_cannot_use_outage_exception(self) -> None:
        with self.assertRaises(ValueError):
            validate_outage(
                self.jobs,
                self.ci,
                OUTAGE_ERROR + " Uploading distributions",
                "source-commit",
            )

    def test_ci_for_other_commit_is_insufficient(self) -> None:
        with self.assertRaises(ValueError):
            validate_outage(self.jobs, self.ci, OUTAGE_ERROR, "different-commit")

    @patch("validate_python_release_source.subprocess.run")
    def test_api_failure_reports_reason_without_token(self, run) -> None:
        run.return_value = subprocess.CompletedProcess(
            [], 1, "", "gh: HTTP 403 credential secret-token"
        )
        with (
            patch.dict("os.environ", {"GH_TOKEN": "secret-token"}),
            self.assertRaisesRegex(RuntimeError, r"HTTP 403 credential \*\*\*"),
        ):
            gh("repos/test/actions/jobs/1/logs", raw=True)


if __name__ == "__main__":
    unittest.main()

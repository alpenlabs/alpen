#!/usr/bin/env python3
"""
Send test flakiness metrics to Grafana Cloud using CI matrix data only.

Uses GitHub API to fetch all matrix job results and sends via InfluxDB line protocol.

Environment variables:
  GRAFANA_CLOUD_INFLUX_URL: InfluxDB endpoint (e.g., https://influx-prod-43-prod-ap-south-1.grafana.net/api/v1/push/influx/write)
  GRAFANA_CLOUD_INFLUX_USER: InfluxDB user ID
  GRAFANA_CLOUD_API_KEY: API key for authentication
  GITHUB_TOKEN: GitHub token for API access
  GITHUB_REPOSITORY: Repository (e.g., "owner/repo")
  GITHUB_RUN_ID: Workflow run ID
  GITHUB_SHA: Commit SHA
  GITHUB_REF: Branch name
  GITHUB_ACTOR: User who triggered the workflow
"""

import os
import sys
import time
from typing import Dict, List

try:
    import requests
except ImportError:
    print("ERROR: requests library not found. Install with: pip install requests")
    sys.exit(1)

# GitHub API configuration
GITHUB_TOKEN = os.getenv("GITHUB_TOKEN", "")
GITHUB_REPOSITORY = os.getenv("GITHUB_REPOSITORY", "")
GITHUB_RUN_ID = os.getenv("GITHUB_RUN_ID", "")

# Grafana Cloud configuration
INFLUX_URL = os.getenv("GRAFANA_CLOUD_INFLUX_URL", "").rstrip("/")
INFLUX_USER = os.getenv("GRAFANA_CLOUD_INFLUX_USER", "")
API_KEY = os.getenv("GRAFANA_CLOUD_API_KEY", "")

# Git context
GIT_SHA = os.getenv("GITHUB_SHA", "unknown")
GIT_BRANCH = os.getenv("GITHUB_REF", "unknown")
GIT_ACTOR = os.getenv("GITHUB_ACTOR", "unknown")


def fetch_workflow_jobs() -> List[Dict]:
    """Fetch all jobs from the workflow run using GitHub API with pagination."""
    if not GITHUB_TOKEN:
        print("ERROR: GITHUB_TOKEN not set")
        return []

    if not GITHUB_REPOSITORY:
        print("ERROR: GITHUB_REPOSITORY not set")
        return []

    if not GITHUB_RUN_ID:
        print("ERROR: GITHUB_RUN_ID not set")
        return []

    print(f"Fetching workflow jobs from GitHub API...")
    print(f"   Repository: {GITHUB_REPOSITORY}")
    print(f"   Run ID: {GITHUB_RUN_ID}")

    all_jobs = []
    page = 1
    per_page = 100

    try:
        while True:
            url = f"https://api.github.com/repos/{GITHUB_REPOSITORY}/actions/runs/{GITHUB_RUN_ID}/jobs?per_page={per_page}&page={page}"

            response = requests.get(
                url,
                headers={
                    "Authorization": f"Bearer {GITHUB_TOKEN}",
                    "Accept": "application/vnd.github.v3+json"
                },
                timeout=30
            )

            if response.status_code != 200:
                print(f"ERROR: Failed to fetch jobs: {response.status_code}")
                print(f"   Response: {response.text}")
                return all_jobs

            data = response.json()
            jobs = data.get("jobs", [])

            if not jobs:
                break

            all_jobs.extend(jobs)
            print(f"   Fetched page {page}: {len(jobs)} jobs")

            # Check if there are more pages
            if len(jobs) < per_page:
                break

            page += 1

        print(f"SUCCESS: Fetched {len(all_jobs)} total jobs from API")
        return all_jobs

    except Exception as e:
        print(f"ERROR: Error fetching jobs: {e}")
        return all_jobs


def extract_test_results(jobs: List[Dict]) -> List[Dict]:
    """
    Extract test results from matrix jobs.

    Matrix jobs have names like: "Test bridge/bridge_deposit_happy"
    We extract the test name and status (success/failure) from the job.
    """
    results = []

    for job in jobs:
        job_name = job.get("name", "")
        conclusion = job.get("conclusion", "")

        # Only process matrix test jobs (skip lint, discover-tests, etc.)
        # Job names from workflow_call appear as: "functional-tests / Test bridge/bridge_deposit_happy"
        if " / Test " not in job_name:
            continue

        # Extract test name from job name: "functional-tests / Test bridge/bridge_deposit_happy" -> "bridge/bridge_deposit_happy"
        test_name = job_name.split(" / Test ", 1)[1].strip()

        # Map GitHub conclusion to our status
        if conclusion == "success":
            status = "passed"
        elif conclusion == "failure":
            status = "failed"
        elif conclusion == "cancelled":
            status = "cancelled"
        elif conclusion == "skipped":
            status = "skipped"
        else:
            status = "unknown"

        results.append({
            "test_name": test_name,
            "status": status,
            "conclusion": conclusion,
        })

    print(f"Extracted {len(results)} test results")

    # Show summary
    passed = sum(1 for r in results if r["status"] == "passed")
    failed = sum(1 for r in results if r["status"] == "failed")
    print(f"   Passed: {passed}")
    print(f"   Failed: {failed}")

    if failed > 0:
        print(f"   Failed tests:")
        for r in results:
            if r["status"] == "failed":
                print(f"     - {r['test_name']}")

    return results


def escape_influx_value(value: str) -> str:
    """Escape special characters for InfluxDB line protocol."""
    if not value:
        return "unknown"
    # Escape spaces, commas, and equals signs in tag values
    value = value.replace(" ", "\\ ").replace(",", "\\,").replace("=", "\\=")
    return value


def convert_to_influx_line_protocol(results: List[Dict]) -> str:
    """
    Convert test results to InfluxDB line protocol.

    Format: measurement,tag1=value1,tag2=value2 field1=value1,field2=value2 timestamp
    Example: test_result,test=bridge_deposit,status=passed count=1i,failures=0i 1704629483000000000
    """
    if not results:
        return ""

    timestamp_ns = int(time.time() * 1_000_000_000)
    lines = []

    branch = escape_influx_value(GIT_BRANCH.replace("refs/heads/", ""))
    actor = escape_influx_value(GIT_ACTOR)

    for result in results:
        test_name = escape_influx_value(result["test_name"])
        status = escape_influx_value(result["status"])

        # Build tags
        tags = f"test={test_name},status={status},branch={branch},actor={actor}"

        # Build fields
        failures = 1 if status == "failed" else 0
        fields = f"count=1i,failures={failures}i"

        # Combine into line protocol
        line = f"test_result,{tags} {fields} {timestamp_ns}"
        lines.append(line)

    return "\n".join(lines)


def send_to_grafana(metrics: str) -> bool:
    """Send metrics to Grafana Cloud using InfluxDB line protocol."""
    if not INFLUX_URL:
        print("ERROR: GRAFANA_CLOUD_INFLUX_URL not set")
        return False

    if not INFLUX_USER:
        print("ERROR: GRAFANA_CLOUD_INFLUX_USER not set")
        return False

    if not API_KEY:
        print("ERROR: GRAFANA_CLOUD_API_KEY not set")
        return False

    if not metrics:
        print("WARNING: No metrics to send")
        return False

    print(f"Sending metrics to Grafana Cloud...")
    print(f"   Endpoint: {INFLUX_URL}")
    print(f"   Metrics lines: {len(metrics.splitlines())}")

    try:
        response = requests.post(
            INFLUX_URL,
            auth=(INFLUX_USER, API_KEY),
            headers={"Content-Type": "text/plain"},
            data=metrics,
            timeout=30
        )

        if response.status_code in [200, 204]:
            print("SUCCESS: Metrics sent successfully!")
            return True
        else:
            print(f"ERROR: Failed to send metrics: {response.status_code}")
            print(f"   Response: {response.text}")
            return False

    except Exception as e:
        print(f"ERROR: Error sending metrics: {e}")
        return False


def main():
    print("Test Metrics Collector")
    print("=" * 50)

    # Fetch workflow jobs from GitHub API
    jobs = fetch_workflow_jobs()

    if not jobs:
        print("WARNING: No jobs found, exiting")
        return 0

    # Extract test results from matrix jobs
    results = extract_test_results(jobs)

    if not results:
        print("WARNING: No test results found, exiting")
        return 0

    # Convert to InfluxDB line protocol
    print("\nConverting to InfluxDB line protocol...")
    metrics = convert_to_influx_line_protocol(results)

    if not metrics:
        print("WARNING: No metrics generated")
        return 0

    # Send to Grafana Cloud
    print("\nSending to Grafana Cloud...")
    success = send_to_grafana(metrics)

    if success:
        print("\nSUCCESS: Test metrics collection complete!")
        return 0
    else:
        print("\nERROR: Test metrics collection failed")
        return 1


if __name__ == "__main__":
    sys.exit(main())

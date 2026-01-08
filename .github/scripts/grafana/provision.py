#!/usr/bin/env python3
"""
Automatically provision Grafana dashboard via API.

Environment variables:
  GRAFANA_URL: Grafana instance URL (e.g., https://your-stack.grafana.net)
  GRAFANA_SERVICE_ACCOUNT_TOKEN: Grafana service account token with Editor permissions
"""

import argparse
import json
import os
import sys

try:
    import requests
except ImportError:
    print("requests library not found. Install with: pip install requests")
    sys.exit(1)

GRAFANA_URL = os.getenv("GRAFANA_URL", "").rstrip("/")
GRAFANA_TOKEN = os.getenv("GRAFANA_SERVICE_ACCOUNT_TOKEN", "")


def load_dashboard_json(file_path: str) -> dict:
    """Load dashboard JSON from file."""
    try:
        with open(file_path, 'r') as f:
            return json.load(f)
    except Exception as e:
        print(f"Failed to load dashboard JSON: {e}")
        sys.exit(1)


def find_prometheus_datasource_uid(grafana_url: str, token: str) -> str:
    """Find the UID of the Prometheus datasource (excluding usage datasource)."""
    try:
        response = requests.get(
            f"{grafana_url}/api/datasources",
            headers={"Authorization": f"Bearer {token}"},
            timeout=10
        )

        if response.status_code != 200:
            print(f"Failed to get datasources: {response.status_code}")
            print(f"   Response: {response.text}")
            return None

        datasources = response.json()

        # Look for Prometheus or Mimir datasource, but exclude usage datasource
        # Prefer datasources with "prom" in the name (user's metrics), not "usage" (billing metrics)
        prom_datasources = []
        for ds in datasources:
            if ds.get("type") in ["prometheus", "prometheus-mimir"]:
                name = ds.get("name", "")
                uid = ds.get("uid")

                # Skip usage/billing datasources
                if "usage" in name.lower():
                    print(f"Skipping usage datasource: {name}")
                    continue

                prom_datasources.append({"name": name, "uid": uid})

        # Prefer datasource with "prom" in name
        for ds in prom_datasources:
            if "prom" in ds["name"].lower():
                print(f"Found Prometheus datasource: {ds['name']} (UID: {ds['uid']})")
                return ds["uid"]

        # Fallback to first non-usage datasource
        if prom_datasources:
            ds = prom_datasources[0]
            print(f"Found Prometheus datasource: {ds['name']} (UID: {ds['uid']})")
            return ds["uid"]

        print("No Prometheus datasource found")
        return None

    except Exception as e:
        print(f"Error finding datasource: {e}")
        return None


def replace_datasource_placeholder(dashboard: dict, datasource_uid: str) -> dict:
    """Replace ${datasource} placeholder with actual datasource UID."""
    dashboard_str = json.dumps(dashboard)
    dashboard_str = dashboard_str.replace("${datasource}", datasource_uid)
    dashboard_str = dashboard_str.replace('"uid": "prometheus"', f'"uid": "{datasource_uid}"')
    return json.loads(dashboard_str)


def check_dashboard_exists(grafana_url: str, token: str, dashboard_uid: str) -> bool:
    """Check if dashboard already exists."""
    try:
        response = requests.get(
            f"{grafana_url}/api/dashboards/uid/{dashboard_uid}",
            headers={"Authorization": f"Bearer {token}"},
            timeout=10
        )
        return response.status_code == 200
    except Exception:
        return False


def provision_dashboard(grafana_url: str, token: str, dashboard_json: dict) -> bool:
    """Provision dashboard via Grafana API (idempotent - creates if not exists, updates if exists)."""
    dashboard_uid = dashboard_json.get("uid", "test-flakiness")

    # Check if dashboard already exists
    print(f"Checking if dashboard exists (UID: {dashboard_uid})...")
    exists = check_dashboard_exists(grafana_url, token, dashboard_uid)

    if exists:
        print("   Dashboard already exists")
    else:
        print("   Dashboard does not exist")

    # Wrap dashboard in the required format
    payload = {
        "dashboard": dashboard_json,
        "overwrite": True,  # Update if exists, create if not
        "message": "Provisioned by CI"
    }

    try:
        action = "Updating" if exists else "Creating"
        print(f"{action} dashboard...")

        response = requests.post(
            f"{grafana_url}/api/dashboards/db",
            headers={
                "Authorization": f"Bearer {token}",
                "Content-Type": "application/json"
            },
            json=payload,
            timeout=30
        )

        if response.status_code in [200, 201]:
            result = response.json()
            dashboard_url = result.get("url", "")
            dashboard_uid = result.get("uid", "")
            status = result.get("status", "")

            if status == "success":
                print(f"SUCCESS: Dashboard provisioned!")
            else:
                print(f"SUCCESS: Dashboard operation completed")

            print(f"   UID: {dashboard_uid}")
            print(f"   URL: {grafana_url}{dashboard_url}")
            return True
        elif response.status_code == 412:
            # Precondition failed - dashboard unchanged
            print(f"   Dashboard already up to date (no changes needed)")
            print(f"   URL: {grafana_url}/d/{dashboard_uid}/test-flakiness-tracker")
            return True
        else:
            print(f"ERROR: Failed to provision dashboard: {response.status_code}")
            print(f"   Response: {response.text}")
            return False

    except Exception as e:
        print(f"ERROR: Exception during provisioning: {e}")
        return False


def main():
    parser = argparse.ArgumentParser(description="Provision Grafana dashboard")
    parser.add_argument(
        "--dashboard-file",
        default=".github/scripts/grafana/dashboard.json",
        help="Path to dashboard JSON file"
    )

    args = parser.parse_args()

    # Validate environment variables
    if not GRAFANA_URL:
        print("GRAFANA_URL not set")
        print("Example: https://your-stack.grafana.net")
        return 1

    if not GRAFANA_TOKEN:
        print("GRAFANA_SERVICE_ACCOUNT_TOKEN not set")
        print("   Create a service account token in Grafana:")
        print("   1. Go to Administration → Service Accounts")
        print("   2. Create new service account with Editor role")
        print("   3. Generate token")
        print("   4. Add to GitHub Secrets")
        return 1

    print("Provisioning Grafana Dashboard")
    print("=" * 50)
    print(f"Grafana URL: {GRAFANA_URL}")
    print(f"Dashboard file: {args.dashboard_file}")
    print()

    # Load dashboard JSON
    print("Loading dashboard JSON...")
    dashboard = load_dashboard_json(args.dashboard_file)
    print(f"   Title: {dashboard.get('title', 'Unknown')}")
    print(f"   Panels: {len(dashboard.get('panels', []))}")
    print()

    # Find Prometheus datasource
    print("Finding Prometheus datasource...")
    datasource_uid = find_prometheus_datasource_uid(GRAFANA_URL, GRAFANA_TOKEN)

    if not datasource_uid:
        print()
        print("WARNING: Could not find Prometheus datasource automatically.")
        print("   Dashboard will use default datasource.")
        print("   You may need to manually select datasource after provisioning.")
        print()
    else:
        # Replace datasource placeholder
        print("Configuring datasource...")
        dashboard = replace_datasource_placeholder(dashboard, datasource_uid)
        print()

    # Provision dashboard
    success = provision_dashboard(GRAFANA_URL, GRAFANA_TOKEN, dashboard)

    if success:
        print()
        print("SUCCESS: Dashboard provisioning complete!")
        print()
        print("View your dashboard:")
        print(f"   {GRAFANA_URL}/d/test-flakiness/test-flakiness-tracker")
        return 0
    else:
        print()
        print("ERROR: Dashboard provisioning failed")
        return 1


if __name__ == "__main__":
    sys.exit(main())

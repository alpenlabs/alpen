# Test Flakiness Tracking

Automated test flakiness tracking using Grafana Cloud and CI matrix data.

## Architecture

```
Functional Tests
  └─> Report pass/fail via exit codes
       └─> GitHub stores job results
            └─> Collector workflow (after tests complete)
                 └─> Fetch results from GitHub API
                       └─> Send to Grafana Cloud
                            └─> Dashboard auto-updates
```


## Required GitHub Secrets

Add these in GitHub Settings > Secrets and variables > Actions:

```
GRAFANA_CLOUD_INFLUX_URL=https://influx-prod-XX-XXX.grafana.net/api/v1/push/influx/write
GRAFANA_CLOUD_INFLUX_USER=<your_user_id>
GRAFANA_CLOUD_API_KEY=<your_api_key>
GRAFANA_URL=https://your-stack.grafana.net
GRAFANA_SERVICE_ACCOUNT_TOKEN=<service_account_token>
```

### Getting Credentials

1. **InfluxDB endpoint**: Grafana Cloud > Connections > Add new connection > InfluxDB
2. **User ID**: Found in InfluxDB connection details
3. **API Key**: Grafana Cloud > API Keys > Create API key (Editor role)
4. **Service Account Token**: Grafana > Administration > Service Accounts > Create (Editor role)

## Dashboard Panels

1. **Test Summary** - Shows total test runs and total failures across all time
2. **Failed Tests** - Table showing all tests that have failed with their failure counts


#!/usr/bin/env bash
# The integration-test pattern: judge a non-deterministic AI output.
#
# `ailloy eval` exits 0 on pass, 1 on fail, 2 on usage error, 3 on
# provider error — so it slots straight into any test script or CI job.
set -euo pipefail

# Imagine this is your tool producing a non-deterministic answer:
answer="$(my-tool ask 'Summarize the incident report')"

echo "$answer" | ailloy eval \
  --criteria "mentions the outage start time, the root cause, and at least one follow-up action" \
  --context "input is a summary of incident INC-4711; the report is about a database failover" \
  --json

# Or gate on a score threshold instead of the judge's own verdict:
echo "$answer" | ailloy eval -c "written in professional English" --threshold 0.8

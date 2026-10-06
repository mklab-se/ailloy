#!/usr/bin/env bash
# The integration-test pattern: judge a non-deterministic AI output.
#
# `ailloy eval` exits 0 on pass, 1 on fail, 2 on usage error, 3 on provider
# error, and 4 when the gates pass but an answer is below --min-confidence,
# so it slots straight into any test script or CI job.
set -euo pipefail

# Imagine this is your tool producing a non-deterministic answer:
answer="$(my-tool ask 'Summarize the incident report')"

# One yes/no check, with context for the judge:
echo "$answer" | ailloy eval \
  --yes-no "Does the summary give the outage start time, the root cause, and at least one follow-up action?" \
  --context "input is a summary of incident INC-4711; the report is about a database failover"

# Stricter: require p(yes) >= 0.8 and route uncertain answers to review (exit 4):
set +e
echo "$answer" | ailloy eval --yes-no "Is it written in professional English?" \
  --threshold 0.8 --min-confidence 0.6
code=$?
set -e
if [ "$code" -eq 4 ]; then echo "judge unsure, flag for human review"; elif [ "$code" -ne 0 ]; then exit "$code"; fi

# Many checks in one request (free batching on a TypeSafe node):
cat > /tmp/checks.yaml <<'YAML'
questions:
  root_cause:
    yes_no: "Does the summary name a root cause?"
  tone:
    choice: "What is the tone of the summary?"
    options: { neutral: "factual", alarmist: "exaggerated urgency", dismissive: "downplays impact" }
    expect: [neutral]
  completeness:
    score: "How complete is the summary?"
    levels: [Missing key facts, Partial, Complete]
    min: 1.5
YAML
echo "$answer" | ailloy eval --questions /tmp/checks.yaml --json

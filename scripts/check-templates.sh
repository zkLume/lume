#!/usr/bin/env bash
# AUDIT 2026-04-19 H-15: templates must be materialized scaffolds that
# cp-and-build cleanly. No unrendered `{{placeholders}}`, no colliding
# crate names. Run locally or from CI before publishing a release.
#
# Exit codes: 0 on clean, 1 on violation.

set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
templates="$here/templates"

if [[ ! -d "$templates" ]]; then
    echo "check-templates: no templates/ at $templates" >&2
    exit 1
fi

fail=0

# 1. Reject any remaining `{{placeholder}}` — these were the H-15 footgun.
matches="$(grep -rn '{{' "$templates" 2>/dev/null || true)"
if [[ -n "$matches" ]]; then
    echo "check-templates: templates still carry unrendered placeholders:" >&2
    echo "$matches" >&2
    fail=1
fi

# 2. Assert each domain template's Cargo.toml declares a unique `name`.
#    Post-materialization collisions would break `cargo install` on the
#    published template set.
declare -A seen_names
for toml in "$templates"/*/Cargo.toml; do
    # Read the first `name = "..."` line inside `[package]`.
    name="$(awk '/^\[package\]/{p=1;next} /^\[/{p=0} p && $1=="name"{gsub(/"/,"",$3); print $3; exit}' "$toml")"
    if [[ -z "$name" ]]; then
        echo "check-templates: $toml has no [package].name" >&2
        fail=1
        continue
    fi
    if [[ -n "${seen_names[$name]:-}" ]]; then
        echo "check-templates: duplicate crate name \`$name\` in $toml (also ${seen_names[$name]})" >&2
        fail=1
    else
        seen_names[$name]="$toml"
    fi
done

if (( fail )); then
    exit 1
fi

echo "check-templates: clean (${#seen_names[@]} templates, no placeholders)"

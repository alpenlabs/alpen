#!/usr/bin/env bash

set -euo pipefail

BANNED_CRATES=("borsh" "bincode")

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
ROOT_MANIFEST="${REPO_ROOT}/Cargo.toml"

is_banned_crate() {
    local crate_name="$1"

    for banned_crate in "${BANNED_CRATES[@]}"; do
        if [[ "${crate_name}" == "${banned_crate}" ]]; then
            return 0
        fi
    done

    return 1
}

is_dependency_section() {
    local section_name="$1"

    if [[ "${section_name}" =~ ^(dependencies|dev-dependencies|build-dependencies|workspace\.dependencies)$ ]]; then
        return 0
    fi

    if [[ "${section_name}" =~ ^target\..*\.(dependencies|dev-dependencies|build-dependencies)$ ]]; then
        return 0
    fi

    return 1
}

trim_quotes() {
    local value="$1"

    value="${value#\"}"
    value="${value%\"}"
    value="${value#\'}"
    value="${value%\'}"

    printf "%s" "${value}"
}

extract_workspace_members() {
    local manifest_path="$1"

    awk '
        BEGIN {
            in_workspace = 0
            in_members = 0
        }

        /^[[:space:]]*\[workspace\][[:space:]]*$/ {
            in_workspace = 1
            in_members = 0
            next
        }

        in_workspace && /^[[:space:]]*\[/ {
            in_workspace = 0
            in_members = 0
        }

        in_workspace {
            if (!in_members && $0 ~ /^[[:space:]]*members[[:space:]]*=[[:space:]]*\[/) {
                in_members = 1
            }

            if (in_members) {
                line = $0
                while (match(line, /"([^"]+)"/)) {
                    print substr(line, RSTART + 1, RLENGTH - 2)
                    line = substr(line, RSTART + RLENGTH)
                }

                if ($0 ~ /\]/) {
                    in_members = 0
                }
            }
        }
    ' "${manifest_path}"
}

report_violation() {
    local manifest_path="$1"
    local line_number="$2"
    local section_name="$3"
    local reason="$4"

    local relative_manifest_path="${manifest_path#"${REPO_ROOT}"/}"
    if [[ "${relative_manifest_path}" == "${manifest_path}" ]]; then
        relative_manifest_path="${manifest_path}"
    fi

    printf "%s:%s: %s: %s\n" "${relative_manifest_path}" "${line_number}" "${section_name}" "${reason}"
}

declare -a manifests
manifests=("${ROOT_MANIFEST}")

while IFS= read -r workspace_member; do
    manifests+=("${REPO_ROOT}/${workspace_member}/Cargo.toml")
done < <(extract_workspace_members "${ROOT_MANIFEST}")

violation_count=0

for manifest_path in "${manifests[@]}"; do
    if [[ ! -f "${manifest_path}" ]]; then
        echo "missing manifest for workspace member: ${manifest_path}"
        exit 1
    fi

    mapfile -t manifest_lines < "${manifest_path}"

    active_section=""
    active_dependency_section=""
    active_dependency_name=""

    for line_index in "${!manifest_lines[@]}"; do
        line="${manifest_lines[${line_index}]}"
        line_number=$((line_index + 1))

        if [[ "${line}" =~ ^[[:space:]]*\[([^\]]+)\][[:space:]]*$ ]]; then
            active_section="${BASH_REMATCH[1]}"
            active_dependency_section=""
            active_dependency_name=""

            if is_dependency_section "${active_section}"; then
                active_dependency_section="${active_section}"
            else
                section_parent="${active_section%.*}"
                section_dependency_name="$(trim_quotes "${active_section##*.}")"

                if [[ "${section_parent}" != "${active_section}" ]] && is_dependency_section "${section_parent}"; then
                    active_dependency_section="${section_parent}"
                    active_dependency_name="${section_dependency_name}"

                    if is_banned_crate "${active_dependency_name}"; then
                        report_violation \
                            "${manifest_path}" \
                            "${line_number}" \
                            "${active_dependency_section}" \
                            "direct dependency table for banned crate '${active_dependency_name}'"
                        violation_count=$((violation_count + 1))
                    fi
                fi
            fi

            continue
        fi

        if [[ -z "${active_dependency_section}" ]]; then
            continue
        fi

        if [[ "${line}" =~ ^[[:space:]]*$ ]] || [[ "${line}" =~ ^[[:space:]]*# ]]; then
            continue
        fi

        if [[ -n "${active_dependency_name}" ]]; then
            if [[ "${line}" =~ ^[[:space:]]*package[[:space:]]*=[[:space:]]*\"([^\"]+)\" ]]; then
                package_name="${BASH_REMATCH[1]}"

                if is_banned_crate "${package_name}"; then
                    report_violation \
                        "${manifest_path}" \
                        "${line_number}" \
                        "${active_dependency_section}" \
                        "renamed dependency '${active_dependency_name}' uses banned package '${package_name}'"
                    violation_count=$((violation_count + 1))
                fi
            fi

            continue
        fi

        if [[ "${line}" =~ ^[[:space:]]*([A-Za-z0-9_-]+)[[:space:]]*= ]]; then
            dependency_name="${BASH_REMATCH[1]}"

            if is_banned_crate "${dependency_name}"; then
                report_violation \
                    "${manifest_path}" \
                    "${line_number}" \
                    "${active_dependency_section}" \
                    "direct dependency on banned crate '${dependency_name}'"
                violation_count=$((violation_count + 1))
                continue
            fi

            for banned_crate in "${BANNED_CRATES[@]}"; do
                if [[ "${line}" =~ package[[:space:]]*=[[:space:]]*\"${banned_crate}\" ]]; then
                    report_violation \
                        "${manifest_path}" \
                        "${line_number}" \
                        "${active_dependency_section}" \
                        "renamed dependency '${dependency_name}' uses banned package '${banned_crate}'"
                    violation_count=$((violation_count + 1))
                fi
            done
        fi
    done
done

if [[ "${violation_count}" -gt 0 ]]; then
    echo "found ${violation_count} banned dependency declaration(s)"
    exit 1
fi

echo "no banned direct dependency declarations found"

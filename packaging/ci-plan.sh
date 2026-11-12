#!/usr/bin/env bash
# What one run of the workflow has to do, decided once for all its jobs.
#
#   full     every check and every build: a pull request, a branch, and a tag that has no candidate to publish.
#   promote  a tag on the very commit of an earlier candidate (vX.Y.Z-rcN) that passed: the workflow run of the
#            candidate's tag checked and built everything on this commit and ended in success, and its pre-release holds
#            the files that run built. Those files become the release, byte for byte the ones that were checked and
#            tried, and nothing is built or checked a second time.
#   covered  a branch whose head commit carries a release tag with a run of its own, going or passed: that run checks
#            the commit, and a second run of the same jobs beside it only takes the runners the first one waits for. The
#            jobs of a covered run read as skipped, not as passed; what the commit is worth is in the run of the tag.
#
# Any doubt about a candidate is `full`: the run then checks and builds everything. A tag that does not name the packaged
# version stops the run here, before anything is published.
#
# Reads GITHUB_REF, GITHUB_SHA, GITHUB_REPOSITORY, GITHUB_EVENT_NAME and GITHUB_WORKFLOW as a workflow sets them, and
# writes `mode=` and `candidate=` to GITHUB_OUTPUT (to the terminal when it is not set). With a folder as the argument a
# `promote` leaves the candidate's files there: the files its run built, each the same as the one its pre-release holds.
# Without one nothing is fetched, which is how the decision is tried by hand:
#   GITHUB_REF=refs/tags/v0.71.2 GITHUB_SHA=<commit> GITHUB_REPOSITORY=olegius88/keyswitch GITHUB_EVENT_NAME=push \
#   GITHUB_WORKFLOW='CI and release' packaging/ci-plan.sh
set -euo pipefail
# Names are compared in the order jq sorts them, by code point: a locale would put `keyswitch_` among `KeySwitch-`.
export LC_ALL=C

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
files_dir="${1:-}"
ref="${GITHUB_REF:?}"
commit="${GITHUB_SHA:?}"
repository="${GITHUB_REPOSITORY:?}"
event="${GITHUB_EVENT_NAME:?}"
workflow="${GITHUB_WORKFLOW:?}"
remote="${KS_PLAN_REMOTE:-origin}"
version="$(sed -nE 's/^version = "([^"]+)"/\1/p' "$project_dir/Cargo.toml" | head -n 1)"
# The job that ends the checks of a run: it passes only in a run that checked and built everything.
checks_job="Every line and branch of the program ran"
# The job a candidate has no use for: its files stay out of the APT repository.
apt_job="Publish the signed APT repository"

decide() {
    printf 'mode=%s\ncandidate=%s\n' "$1" "${2:-}" >> "${GITHUB_OUTPUT:-/dev/stdout}"
    printf 'This run is %s%s: %s\n' "$1" "${2:+ of $2}" "$3" >&2
    exit 0
}

# The tags of the remote that point at this commit. An annotated tag is listed twice, as the tag object and, with ^{}
# after its name, as the commit it names; a lightweight tag once, as the commit.
tags_here() {
    git -C "$project_dir" ls-remote --tags "$remote" | awk -v commit="$commit" '
        {
            name = $2
            sub(/^refs\/tags\//, "", name)
            if (sub(/\^\{\}$/, "", name) || !(name in at)) at[name] = $1
        }
        END { for (name in at) if (at[name] == commit) print name }'
}

# The number of a candidate tag of this version, or nothing.
candidate_number() {
    local number="${1#"v$version"-rc}"
    if [[ "$1" == "v$version"-rc* && "$number" =~ ^[0-9]+$ ]]; then
        printf '%d\n' "$((10#$number))"
    fi
}

before=""
case "$ref" in
    refs/tags/v*)
        tag="${ref#refs/tags/}"
        # A release publishes the last candidate on its commit; a candidate, the last one before it.
        before="$(candidate_number "$tag")"
        if [[ "$tag" != "v$version" && -z "$before" ]]; then
            printf 'Tag %s does not name the packaged version %s\n' "$tag" "$version" >&2
            exit 1
        fi
        ;;
    refs/heads/*) ;;
    *) decide full "" "not a branch and not a release tag" ;;
esac
[[ "$event" == push ]] || decide full "" "not a push"
here="$(tags_here)" || decide full "" "the tags of the remote could not be read"

if [[ "$ref" == refs/heads/* ]]; then
    while IFS= read -r tag; do
        if [[ "$tag" == "v$version" || -n "$(candidate_number "$tag")" ]]; then
            # The tag has a run on this commit that has not ended otherwise than in success.
            of_tag="$(gh run list --repo "$repository" --workflow "$workflow" --branch "$tag" --event push --limit 1 --json status,conclusion,headSha)" \
                || decide full "" "the workflow run of $tag could not be read"
            if jq -e --arg commit "$commit" 'length == 1 and .[0].headSha == $commit and (.[0].status != "completed" or .[0].conclusion == "success")' <<< "$of_tag" > /dev/null; then
                decide covered "$tag" "the run of that tag checks this commit"
            fi
        fi
    done <<< "$here"
    decide full "" "a branch with no release tag that has a run on its head commit"
fi

# The earlier candidates on this commit, the last one first.
candidates="$(while IFS= read -r name; do
    number="$(candidate_number "$name")"
    if [[ -n "$number" ]] && { [[ -z "$before" ]] || ((number < before)); }; then
        printf '%d %s\n' "$number" "$name"
    fi
done <<< "$here" | sort -rn | cut -d ' ' -f 2)"
[[ -n "$candidates" ]] || decide full "" "no earlier candidate of $version is on this commit"

# Every other run of this commit has ended, and in success: one that failed beside one that passed is a doubt, and so is
# one that still goes on.
others="$(gh run list --repo "$repository" --workflow "$workflow" --commit "$commit" --limit 50 --json databaseId,status,conclusion)" \
    || decide full "" "the workflow runs of this commit could not be read"
jq -e --arg self "${GITHUB_RUN_ID:-}" 'all(.[]; (.databaseId | tostring) == $self or (.status == "completed" and .conclusion == "success"))' <<< "$others" > /dev/null \
    || decide full "" "another workflow run of this commit failed or still goes on"

# The last candidate whose run checked and built everything on this commit, and passed. A candidate that itself
# published the files of an earlier one is passed over for that earlier one.
candidate=""
for name in $candidates; do
    runs="$(gh run list --repo "$repository" --workflow "$workflow" --branch "$name" --event push --limit 1 --json databaseId,status,conclusion,headSha)" \
        || decide full "" "the workflow run of $name could not be read"
    jq -e --arg commit "$commit" 'length == 1 and .[0].status == "completed" and .[0].conclusion == "success" and .[0].headSha == $commit' <<< "$runs" > /dev/null \
        || continue
    run="$(jq -r '.[0].databaseId' <<< "$runs")"
    jobs="$(gh run view "$run" --repo "$repository" --json jobs)" || decide full "" "the jobs of the run of $name could not be read"
    if jq -e --arg checks "$checks_job" --arg apt "$apt_job" '
        (.jobs | any(.name == $checks and .conclusion == "success"))
        and (.jobs | all(.conclusion == "success" or (.conclusion == "skipped" and .name == $apt)))' <<< "$jobs" > /dev/null; then
        candidate="$name"
        break
    fi
done
[[ -n "$candidate" ]] || decide full "" "no candidate on this commit has a run that checked and built everything and passed"

expected=(
    "KeySwitch-Setup-$version-x64.exe" "KeySwitch-Setup-$version-arm64.exe" "KeySwitch-Setup-$version-x86.exe"
    "KeySwitch-$version-windows-x64.zip" "KeySwitch-$version-windows-arm64.zip" "KeySwitch-$version-windows-x86.zip"
    "keyswitch_${version}_amd64.deb" "KeySwitch-$version-macos-arm64.zip" "KeySwitch-$version-macos-x86_64.zip"
)
release="$(gh api "repos/$repository/releases/tags/$candidate")" || decide full "" "the pre-release of $candidate could not be read"
wanted="$(printf '%s\n' "${expected[@]}" SHA256SUMS | sort)"
jq -e --arg wanted "$wanted" '(.draft | not) and .prerelease and (([.assets[].name] | sort | join("\n")) == $wanted)' <<< "$release" > /dev/null \
    || decide full "" "the pre-release of $candidate does not hold exactly the files of a release"

if [[ -n "$files_dir" ]]; then
    # The files the candidate's run built, as that run kept them: nothing written to the pre-release since can be among them.
    built="$files_dir.built"
    rm -rf -- "$files_dir" "$built"
    mkdir -p -- "$files_dir"
    gh run download "$run" --repo "$repository" --dir "$built" --pattern 'keyswitch-release-*' \
        || decide full "" "the files the run of $candidate built are no longer kept"
    find "$built" -type f -exec mv -- {} "$files_dir/" \;
    rm -rf -- "$built"
    [[ "$(find "$files_dir" -type f -printf '%f\n' | sort)" == "$(printf '%s\n' "${expected[@]}" | sort)" ]] \
        || decide full "" "the run of $candidate did not build exactly the files of a release"
    # Each is the file the pre-release holds, the one that was downloaded and tried.
    published="$(jq -r '.assets[] | select(.name != "SHA256SUMS") | "\(.digest // "" | sub("^sha256:"; ""))  \(.name)"' <<< "$release" | sort -k 2)"
    built_sums="$(cd -- "$files_dir" && sha256sum -- * | sort -k 2)"
    [[ "$built_sums" == "$published" ]] || decide full "" "a file of the pre-release of $candidate is not the file its run built"
fi
decide promote "$candidate" "its run checked and built these files on this commit"

# The tests of ks-train, dealt out for the runners of CI. Sourced by packaging/coverage.sh, which measures them, and by
# packaging/stable-tests.sh, which runs them on the stable compiler: both go through the same groups.
#
# The integration tests of ks-train take most of a whole run of the tests. The longest are dealt out by name, into groups
# that take a four-core runner about ten minutes each (the time of each file is in the log of any job that runs it, in
# its `finished in` line); every other test of ks-train is in `rest_tests`, so a new test file is run without being
# listed here.
long_1=(models_kept)
long_2=(fit_small typing)
long_3=(corpora cycle)
long_4=(fit_whole curricula)

# The groups name the targets of ks-train one by one, so this stops on a target they would not run as a plain
# `cargo test` does (an example, a bench, a target with `test = false`, a library without doc tests), on a listed test that
# is gone and on a test listed twice. It is checked before anything runs, in each part and in a whole run alike, so that a
# list gone stale stops a local run before it reaches CI.
found="$(cargo metadata --no-deps --format-version 1 | jq -r '
    .packages[] | select(.name == "ks-train") | .targets[] | select(.kind != ["custom-build"])
    | if (.kind - ["lib", "bin", "test"]) != [] or .test != true or (.kind == ["lib"] and .doctest != true)
      then error("packaging/test-parts.sh does not deal out the target \(.name) of ks-train (\(.kind | join(",")))")
      elif .kind == ["test"] then .name
      else empty end')"
trainer_tests=()
if [[ -n "$found" ]]; then
    mapfile -t trainer_tests <<< "$found"
fi
declare -A listed=() known=()
problems=()
for name in "${long_1[@]}" "${long_2[@]}" "${long_3[@]}" "${long_4[@]}"; do
    if [[ -n "${listed[$name]:-}" ]]; then
        problems+=("$name is listed twice")
    fi
    listed[$name]=1
done
rest_tests=()
for name in "${trainer_tests[@]}"; do
    known[$name]=1
    if [[ -z "${listed[$name]:-}" ]]; then
        rest_tests+=("$name")
    fi
done
for name in "${!listed[@]}"; do
    if [[ -z "${known[$name]:-}" ]]; then
        problems+=("$name is not a test of ks-train")
    fi
done
if ((${#problems[@]})); then
    printf 'packaging/test-parts.sh: %s\n' "${problems[@]}" >&2
    exit 1
fi

# Sourced by packaging/build-macos.sh: runs a command that signs, and runs it again
# while it fails only because Apple's timestamp service did not answer.
#
# Signing with the hardened runtime asks Apple's timestamp service for a secure
# timestamp. When the service does not answer, codesign fails with one of the
# messages below and a later attempt succeeds: on 09.10.2026 the macos-15 arm64
# build of 0.47.1 failed so inside Nuitka's signing while the same commit had
# built and signed on main minutes before. Any other failure stops the build at
# once, as it did before.

macos_timestamp_failure='A timestamp was expected but was not found|The timestamp service is not available'
macos_sign_attempts="${KEYSWITCH_SIGN_ATTEMPTS:-3}"
macos_sign_retry_seconds="${KEYSWITCH_SIGN_RETRY_SECONDS:-30}"

# run_signing COMMAND [ARGUMENT...]: the command's output is shown as it runs;
# its exit status is returned once it succeeds, fails otherwise, or has failed
# for want of a timestamp on every attempt.
run_signing() {
    local attempt=1 log status
    log="$(mktemp)"
    while true; do
        set +e
        "$@" 2>&1 | tee "$log"
        status=${PIPESTATUS[0]}
        set -e
        if [[ "$status" -eq 0 ]]; then
            rm -f "$log"
            return 0
        fi
        if [[ "$attempt" -ge "$macos_sign_attempts" ]] || ! grep -Eq "$macos_timestamp_failure" "$log"; then
            rm -f "$log"
            return "$status"
        fi
        printf "Apple's timestamp service did not answer (attempt %s of %s); signing again in %s s.\n" \
            "$attempt" "$macos_sign_attempts" "$macos_sign_retry_seconds" >&2
        sleep "$macos_sign_retry_seconds"
        attempt=$((attempt + 1))
    done
}

#!/bin/zsh
# The real test on a Mac. Open it in a Terminal that holds the Accessibility permission: the test and the program it starts
# inherit the permission from it. Writes the result beside the build, and closes when done.
project="${0:A:h:h}"
export KS_E2E_BINARY="$project/dist/macos-native/KeySwitch.app/Contents/MacOS/KeySwitch"
export PATH="$HOME/.cargo/bin:$PATH"
caffeinate -u -t 900 &
keep_awake=$!
cd "$project"
{ cargo test --release -p ks-mac --test e2e_mac; print "== exit=$?"; } > "$project/e2e-mac.log" 2>&1
kill $keep_awake 2>/dev/null
exit

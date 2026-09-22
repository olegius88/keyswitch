#!/usr/bin/env bash
# Create the OpenPGP key that signs the KeySwitch APT repository.
#
# The private half belongs in the repository secrets that the release workflow
# reads; the public half is committed as packaging/keyswitch-archive-keyring.asc
# and is shipped inside the Debian package, so APT trusts it for this
# repository alone. Keep the generated directory outside Git.
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target="${1:-}"
identity="${2:-KeySwitch APT repository <sh.oleg@list.ru>}"
expiry="10y"

if [[ -z "$target" ]]; then
    printf 'Usage: %s <output-directory> [uid]\n' "$0" >&2
    exit 2
fi
if [[ -e "$target" ]]; then
    printf 'Refusing to overwrite an existing signing-key directory: %s\n' \
        "$target" >&2
    exit 1
fi
for required_command in gpg openssl; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'Missing signing-key command: %s\n' "$required_command" >&2
        exit 1
    fi
done

mkdir -p "$target"
chmod 0700 "$target"
export GNUPGHOME="$target/gnupg"
mkdir -p "$GNUPGHOME"
chmod 0700 "$GNUPGHOME"

passphrase="$(openssl rand -base64 32)"
printf '%s\n' "$passphrase" > "$target/passphrase.txt"
chmod 0600 "$target/passphrase.txt"

gpg --batch --yes --pinentry-mode loopback --passphrase "$passphrase" \
    --quick-generate-key "$identity" rsa4096 sign "$expiry"
fingerprint="$(gpg --batch --with-colons --list-keys \
    | awk -F: '$1 == "fpr" { print $10; exit }')"
test -n "$fingerprint"

gpg --batch --yes --pinentry-mode loopback --passphrase "$passphrase" \
    --armor --export-secret-keys "$fingerprint" > "$target/private-key.asc"
chmod 0600 "$target/private-key.asc"
gpg --batch --yes --armor --export "$fingerprint" > "$target/public-key.asc"
chmod 0644 "$target/public-key.asc"
printf '%s\n' "$fingerprint" > "$target/fingerprint.txt"

cat <<REPORT
Created the KeySwitch APT signing key.

  fingerprint : $fingerprint
  public key  : $target/public-key.asc
  private key : $target/private-key.asc
  passphrase  : $target/passphrase.txt

Publish the public half and keep the private half secret:

  cp $target/public-key.asc $project_dir/packaging/keyswitch-archive-keyring.asc
  gh secret set KEYSWITCH_APT_SIGNING_KEY < $target/private-key.asc
  gh secret set KEYSWITCH_APT_SIGNING_PASSPHRASE < $target/passphrase.txt
REPORT

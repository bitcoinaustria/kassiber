#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

usage() {
  cat >&2 <<'EOF'
Usage: build-rpm-repository.sh --input DIR --output DIR \
  [--architecture ARCH ...] (--signing-key FINGERPRINT | --unsigned)

Build a new Kassiber DNF repository directory from binary RPM packages. The
output path must not already exist. Signed mode signs every RPM plus
repodata/repomd.xml; --unsigned is only for local tests.
EOF
}

die() {
  echo "$*" >&2
  exit 2
}

input=""
output=""
signing_key=""
unsigned=false
architectures=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --input) input="${2:-}"; shift 2 ;;
    --output) output="${2:-}"; shift 2 ;;
    --architecture) architectures+=("${2:-}"); shift 2 ;;
    --signing-key) signing_key="${2:-}"; shift 2 ;;
    --unsigned) unsigned=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown argument: $1" ;;
  esac
done

[ -n "$input" ] || die "--input is required"
[ -d "$input" ] || die "Input directory does not exist: $input"
[ -n "$output" ] || die "--output is required"
[ ! -e "$output" ] || die "Output path already exists: $output"
if [ "$unsigned" = true ] && [ -n "$signing_key" ]; then
  die "Use either --signing-key or --unsigned, not both"
fi
if [ "$unsigned" != true ] && [ -z "$signing_key" ]; then
  die "--signing-key is required; use --unsigned only for a local test"
fi
if [ -n "$signing_key" ]; then
  case "$signing_key" in
    *[!0-9A-Fa-f]*|"") die "Signing key must be a full hexadecimal fingerprint" ;;
  esac
  if [ "${#signing_key}" -ne 40 ] && [ "${#signing_key}" -ne 64 ]; then
    die "Signing key must be a full 40- or 64-character fingerprint"
  fi
fi
if [ "${#architectures[@]}" -eq 0 ]; then
  architectures=(x86_64)
fi
for architecture in "${architectures[@]}"; do
  case "$architecture" in
    x86_64|aarch64) ;;
    *) die "Unsupported RPM architecture: $architecture" ;;
  esac
done

for command in awk cmp cpio createrepo_c gpg rpm rpm2cpio rpmkeys; do
  command -v "$command" >/dev/null 2>&1 || die "$command is required"
done
if [ -n "$signing_key" ]; then
  command -v rpmsign >/dev/null 2>&1 || die "rpmsign is required for signing"
  gpg --batch --list-secret-keys "$signing_key" >/dev/null 2>&1 \
    || die "Signing key is not available in the active GnuPG home: $signing_key"
fi

mapfile -d '' packages < <(
  find "$input" -maxdepth 1 -type f -name '*.rpm' ! -name '*.src.rpm' -print0 \
    | sort -z
)
[ "${#packages[@]}" -gt 0 ] || die "No binary RPM packages found in $input"

mkdir -p "$(dirname "$output")"
stage="$(mktemp -d "${output}.tmp.XXXXXX")"
verification=""
cleanup() {
  if [ -n "${stage:-}" ] && [ -d "$stage" ]; then
    rm -rf "$stage"
  fi
  if [ -n "${verification:-}" ] && [ -d "$verification" ]; then
    rm -rf "$verification"
  fi
}
trap cleanup EXIT
mkdir -p "$stage/packages"
if [ -n "$signing_key" ]; then
  verification="$(mktemp -d "${output}.verify.XXXXXX")"
  mkdir "$verification/rpmdb"
  gpg --batch --armor --export "$signing_key" \
    > "$verification/signing-key.asc"
  [ -s "$verification/signing-key.asc" ] \
    || die "Could not export the RPM signing public key"
  rpm --dbpath "$verification/rpmdb" --initdb
  rpmkeys --dbpath "$verification/rpmdb" \
    --import "$verification/signing-key.asc"
fi

for package_path in "${packages[@]}"; do
  package_name="$(rpm -qp --queryformat '%{NAME}' "$package_path")"
  package_version="$(rpm -qp --queryformat '%{VERSION}' "$package_path")"
  package_release="$(rpm -qp --queryformat '%{RELEASE}' "$package_path")"
  package_architecture="$(rpm -qp --queryformat '%{ARCH}' "$package_path")"
  case "$package_name" in
    kassiber|kassiber-cli) ;;
    *) die "Unexpected package in Kassiber repository input: $package_name" ;;
  esac
  architecture_allowed=false
  for architecture in "${architectures[@]}"; do
    if [ "$package_architecture" = "$architecture" ] \
        || [ "$package_architecture" = "noarch" ]; then
      architecture_allowed=true
      break
    fi
  done
  if [ "$architecture_allowed" != true ]; then
    die "Package architecture $package_architecture was not declared"
  fi
  canonical_filename="${package_name}-${package_version}-${package_release}.${package_architecture}.rpm"
  destination="$stage/packages/$canonical_filename"
  [ ! -e "$destination" ] || die "Duplicate RPM identity: $canonical_filename"
  install -m 0644 "$package_path" "$destination"
  if [ -n "$signing_key" ]; then
    rpmsign --addsign \
      --define "_openpgp_sign_id $signing_key" \
      --define "_gpg_name $signing_key" \
      --define "_gpg_path ${GNUPGHOME:-$HOME/.gnupg}" \
      "$destination"
    rpmkeys --dbpath "$verification/rpmdb" \
      --checksig --verbose "$destination"
  fi
done

createrepo_c --checksum sha256 --unique-md-filenames "$stage"
if [ -n "$signing_key" ]; then
  gpg --batch --yes --local-user "$signing_key" --digest-algo SHA256 \
    --armor --detach-sign \
    --output "$stage/repodata/repomd.xml.asc" \
    "$stage/repodata/repomd.xml"
fi

chmod -R u=rwX,go=rX "$stage"
mv "$stage" "$output"
stage=""
echo "Built Kassiber DNF repository: $output"

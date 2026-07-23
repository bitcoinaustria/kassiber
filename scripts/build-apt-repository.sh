#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

usage() {
  cat >&2 <<'EOF'
Usage: build-apt-repository.sh --input DIR --output DIR --suite SUITE \
  --architecture ARCH [--architecture ARCH ...] \
  [--component COMPONENT] [--origin ORIGIN] [--label LABEL] \
  [--valid-for-days DAYS] [--release-epoch UNIX_SECONDS] \
  [--not-automatic] [--but-automatic-upgrades] \
  (--signing-key FINGERPRINT | --unsigned)

Build a new Kassiber APT repository directory from Debian packages. The output
path must not already exist. Signing is mandatory unless --unsigned is passed
explicitly for a local test.
EOF
}

die() {
  echo "$*" >&2
  exit 2
}

input=""
output=""
suite=""
component="main"
origin="Kassiber"
label="Kassiber"
valid_for_days="14"
release_epoch=""
signing_key=""
unsigned=false
not_automatic=false
but_automatic_upgrades=false
architectures=()

while [ "$#" -gt 0 ]; do
  case "$1" in
    --input) input="${2:-}"; shift 2 ;;
    --output) output="${2:-}"; shift 2 ;;
    --suite) suite="${2:-}"; shift 2 ;;
    --component) component="${2:-}"; shift 2 ;;
    --origin) origin="${2:-}"; shift 2 ;;
    --label) label="${2:-}"; shift 2 ;;
    --architecture) architectures+=("${2:-}"); shift 2 ;;
    --valid-for-days) valid_for_days="${2:-}"; shift 2 ;;
    --release-epoch) release_epoch="${2:-}"; shift 2 ;;
    --signing-key) signing_key="${2:-}"; shift 2 ;;
    --unsigned) unsigned=true; shift ;;
    --not-automatic) not_automatic=true; shift ;;
    --but-automatic-upgrades) but_automatic_upgrades=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown argument: $1" ;;
  esac
done

[ -n "$input" ] || die "--input is required"
[ -d "$input" ] || die "Input directory does not exist: $input"
[ -n "$output" ] || die "--output is required"
[ ! -e "$output" ] || die "Output path already exists: $output"
[ -n "$suite" ] || die "--suite is required"
[ "${#architectures[@]}" -gt 0 ] || die "At least one --architecture is required"

case "$suite" in
  *[!a-z0-9._-]*|""|-*) die "Invalid suite: $suite" ;;
esac
if [ "$suite" = "." ] || [ "$suite" = ".." ]; then
  die "Invalid suite: $suite"
fi
case "$component" in
  *[!a-z0-9._-]*|""|-*) die "Invalid component: $component" ;;
esac
if [ "$component" = "." ] || [ "$component" = ".." ]; then
  die "Invalid component: $component"
fi
case "$valid_for_days" in
  *[!0-9]*|"") die "--valid-for-days must be an integer" ;;
esac
if [ "$valid_for_days" -lt 1 ] || [ "$valid_for_days" -gt 365 ]; then
  die "--valid-for-days must be between 1 and 365"
fi
if [ -n "$release_epoch" ]; then
  case "$release_epoch" in
    *[!0-9]*) die "--release-epoch must be a non-negative integer" ;;
  esac
fi
if [ "$but_automatic_upgrades" = true ] && [ "$not_automatic" != true ]; then
  die "--but-automatic-upgrades requires --not-automatic"
fi
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
for value in "$origin" "$label"; do
  case "$value" in
    *$'\n'*|*$'\r'*|"") die "Origin and label must be non-empty single-line values" ;;
  esac
done

for command in apt-ftparchive awk cmp dpkg-deb dpkg-scanpackages gzip sha256sum tar; do
  command -v "$command" >/dev/null 2>&1 || die "$command is required"
done
if [ -n "$signing_key" ]; then
  command -v gpg >/dev/null 2>&1 || die "gpg is required for signing"
fi

for architecture in "${architectures[@]}"; do
  case "$architecture" in
    *[!a-z0-9-]*|""|-*|*-) die "Invalid architecture: $architecture" ;;
  esac
done

mapfile -d '' packages < <(
  find "$input" -maxdepth 1 -type f -name '*.deb' -print0 | sort -z
)
[ "${#packages[@]}" -gt 0 ] || die "No Debian packages found in $input"

output_parent="$(dirname "$output")"
mkdir -p "$output_parent"
stage="$(mktemp -d "${output}.tmp.XXXXXX")"
cleanup() {
  if [ -n "${stage:-}" ] && [ -d "$stage" ]; then
    rm -rf "$stage"
  fi
}
trap cleanup EXIT

for package_path in "${packages[@]}"; do
  package_name="$(dpkg-deb -f "$package_path" Package)"
  package_version="$(dpkg-deb -f "$package_path" Version)"
  package_architecture="$(dpkg-deb -f "$package_path" Architecture)"

  case "$package_name" in
    kassiber|kassiber-cli) ;;
    *) die "Unexpected package in Kassiber repository input: $package_name" ;;
  esac

  architecture_allowed=false
  for architecture in "${architectures[@]}"; do
    if [ "$package_architecture" = "$architecture" ] || [ "$package_architecture" = "all" ]; then
      architecture_allowed=true
      break
    fi
  done
  if [ "$architecture_allowed" != true ]; then
    die "Package architecture $package_architecture was not declared with --architecture"
  fi

  filename_version="${package_version//:/_}"
  pool_dir="$stage/pool/$component/k/$package_name"
  destination="$pool_dir/${package_name}_${filename_version}_${package_architecture}.deb"
  mkdir -p "$pool_dir"
  [ ! -e "$destination" ] || die "Duplicate package/version/architecture: $destination"
  cp "$package_path" "$destination"
done

architecture_field="${architectures[*]}"
for architecture in "${architectures[@]}"; do
  index_dir="$stage/dists/$suite/$component/binary-$architecture"
  mkdir -p "$index_dir/by-hash/SHA256"
  (
    cd "$stage"
    dpkg-scanpackages --arch "$architecture" --multiversion "pool/$component"
  ) > "$index_dir/Packages"
  gzip -n -9 -c "$index_dir/Packages" > "$index_dir/Packages.gz"
  for index in "$index_dir/Packages" "$index_dir/Packages.gz"; do
    index_hash="$(sha256sum "$index" | awk '{print $1}')"
    cp "$index" "$index_dir/by-hash/SHA256/$index_hash"
  done
done

if [ -z "$release_epoch" ]; then
  release_epoch="$(date -u +%s)"
fi
release_date="$(date -u -d "@$release_epoch" '+%a, %d %b %Y %H:%M:%S UTC')"
valid_until_epoch="$((release_epoch + valid_for_days * 86400))"
valid_until="$(date -u -d "@$valid_until_epoch" '+%a, %d %b %Y %H:%M:%S UTC')"

release_options=(
  -o "APT::FTPArchive::Release::Origin=$origin"
  -o "APT::FTPArchive::Release::Label=$label"
  -o "APT::FTPArchive::Release::Suite=$suite"
  -o "APT::FTPArchive::Release::Codename=$suite"
  -o "APT::FTPArchive::Release::Architectures=$architecture_field"
  -o "APT::FTPArchive::Release::Components=$component"
  -o "APT::FTPArchive::Release::Description=Kassiber $suite packages"
  -o "APT::FTPArchive::Release::Date=$release_date"
  -o "APT::FTPArchive::Release::Valid-Until=$valid_until"
  -o "APT::FTPArchive::Release::Acquire-By-Hash=yes"
)
if [ "$not_automatic" = true ]; then
  release_options+=( -o "APT::FTPArchive::Release::NotAutomatic=yes" )
fi
if [ "$but_automatic_upgrades" = true ]; then
  release_options+=( -o "APT::FTPArchive::Release::ButAutomaticUpgrades=yes" )
fi

release_dir="$stage/dists/$suite"
(
  cd "$stage"
  apt-ftparchive "${release_options[@]}" release "dists/$suite"
) > "$release_dir/Release"

if [ -n "$signing_key" ]; then
  gpg --batch --list-secret-keys "$signing_key" >/dev/null 2>&1 \
    || die "Signing key is not available in the active GnuPG home: $signing_key"
  gpg --batch --yes --local-user "$signing_key" --digest-algo SHA256 \
    --clearsign --output "$release_dir/InRelease" "$release_dir/Release"
  gpg --batch --yes --local-user "$signing_key" --digest-algo SHA256 \
    --armor --detach-sign --output "$release_dir/Release.gpg" "$release_dir/Release"
fi

chmod -R u=rwX,go=rX "$stage"
mv "$stage" "$output"
stage=""
echo "Built Kassiber APT repository: $output"

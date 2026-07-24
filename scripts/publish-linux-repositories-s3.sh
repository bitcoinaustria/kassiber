#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

usage() {
  echo "Usage: $0 --apt DIR --dnf DIR --suite SUITE --destination s3://BUCKET/PREFIX --base-url URL [--endpoint URL]" >&2
}

die() {
  echo "$*" >&2
  exit 2
}

apt_repository=""
dnf_repository=""
suite=""
destination=""
base_url=""
endpoint=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --apt) apt_repository="${2:-}"; shift 2 ;;
    --dnf) dnf_repository="${2:-}"; shift 2 ;;
    --suite) suite="${2:-}"; shift 2 ;;
    --destination) destination="${2:-}"; shift 2 ;;
    --base-url) base_url="${2:-}"; shift 2 ;;
    --endpoint) endpoint="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown argument: $1" ;;
  esac
done

[ -d "$apt_repository/pool" ] || die "APT pool is missing: $apt_repository"
[ -f "$apt_repository/dists/$suite/InRelease" ] \
  || die "APT InRelease is missing for suite: $suite"
[ -f "$apt_repository/dists/$suite/Release" ] \
  || die "APT Release is missing for suite: $suite"
[ -f "$apt_repository/dists/$suite/Release.gpg" ] \
  || die "APT Release.gpg is missing for suite: $suite"
[ -d "$dnf_repository/packages" ] || die "DNF package directory is missing"
[ -f "$dnf_repository/repodata/repomd.xml" ] || die "DNF repomd.xml is missing"
[ -f "$dnf_repository/repodata/repomd.xml.asc" ] \
  || die "DNF repomd.xml.asc is missing"
case "$destination" in
  s3://?*) ;;
  *) die "Destination must start with s3://" ;;
esac
case "$suite" in
  *[!a-z0-9._-]*|""|.|..) die "Invalid suite: $suite" ;;
esac
case "$base_url" in
  https://?*) ;;
  *) die "Base URL must use HTTPS" ;;
esac
for command in awk aws cmp dirname grep mktemp sha256sum; do
  command -v "$command" >/dev/null 2>&1 || die "$command is required"
done

destination="${destination%/}"
base_url="${base_url%/}"
aws_args=()
if [ -n "$endpoint" ]; then
  aws_args+=(--endpoint-url "$endpoint")
fi

release_dir="$apt_repository/dists/$suite"
by_hash_manifest="$(mktemp)"
mirrorlist=""
cleanup() {
  rm -f "$by_hash_manifest"
  if [ -n "$mirrorlist" ]; then
    rm -f "$mirrorlist"
  fi
}
trap cleanup EXIT

if ! grep -Eq '^Acquire-By-Hash:[[:space:]]+yes[[:space:]]*$' \
    "$release_dir/Release"; then
  die "APT Release must enable Acquire-By-Hash"
fi
if ! awk '
  $0 == "SHA256:" {
    in_sha256 = 1
    next
  }
  in_sha256 && $0 ~ /^[A-Za-z0-9-]+:$/ {
    in_sha256 = 0
  }
  in_sha256 && NF {
    if (NF != 3 || length($1) != 64 || $1 !~ /^[0-9A-Fa-f]+$/) {
      exit 1
    }
    print tolower($1) "\t" $3
    seen = 1
  }
  END {
    if (!seen) {
      exit 1
    }
  }
' "$release_dir/Release" > "$by_hash_manifest"; then
  die "APT Release has no valid SHA256 index manifest"
fi

verified_by_hash=0
while IFS=$'\t' read -r expected_hash relative_path; do
  case "$relative_path" in
    ""|/*|./*|../*|*/../*|*/..|*//*)
      die "APT Release contains an unsafe index path: $relative_path"
      ;;
    */by-hash/*)
      continue
      ;;
  esac
  index_path="$release_dir/$relative_path"
  [ -f "$index_path" ] \
    || die "APT Release references a missing index: $relative_path"
  by_hash_path="$(dirname "$index_path")/by-hash/SHA256/$expected_hash"
  [ -f "$by_hash_path" ] \
    || die "APT by-hash object is missing for: $relative_path"
  actual_hash="$(sha256sum "$by_hash_path" | awk '{print tolower($1)}')"
  [ "$actual_hash" = "$expected_hash" ] \
    || die "APT by-hash checksum mismatch for: $relative_path"
  cmp -s "$index_path" "$by_hash_path" \
    || die "APT mutable and by-hash indices differ for: $relative_path"
  verified_by_hash="$((verified_by_hash + 1))"
done < "$by_hash_manifest"
[ "$verified_by_hash" -gt 0 ] \
  || die "APT Release does not reference any mutable indices"

# Publish immutable payloads and hashed metadata first. APT switches through
# one InRelease object. DNF publishes a complete immutable snapshot and then
# changes its one-object mirrorlist pointer.
aws "${aws_args[@]}" s3 sync \
  "$apt_repository/pool" "$destination/apt/pool"
aws "${aws_args[@]}" s3 sync \
  "$apt_repository/dists/$suite" "$destination/apt/dists/$suite" \
  --exclude "*" --include "*/by-hash/SHA256/*"
aws "${aws_args[@]}" s3 sync \
  "$apt_repository/dists/$suite" "$destination/apt/dists/$suite" \
  --exclude InRelease --exclude Release --exclude Release.gpg \
  --exclude "*/by-hash/*"
# Detached Release/Release.gpg are compatibility copies and cannot switch as
# one object. Atomic clients must use InRelease with Acquire-By-Hash.
aws "${aws_args[@]}" s3 cp \
  "$apt_repository/dists/$suite/Release" \
  "$destination/apt/dists/$suite/Release" \
  --content-type text/plain --cache-control no-cache
aws "${aws_args[@]}" s3 cp \
  "$apt_repository/dists/$suite/Release.gpg" \
  "$destination/apt/dists/$suite/Release.gpg" \
  --content-type application/pgp-signature --cache-control no-cache
aws "${aws_args[@]}" s3 cp \
  "$apt_repository/dists/$suite/InRelease" \
  "$destination/apt/dists/$suite/InRelease" \
  --content-type text/plain --cache-control no-cache

snapshot_id="$(
  sha256sum \
    "$dnf_repository/repodata/repomd.xml" \
    "$dnf_repository/repodata/repomd.xml.asc" \
    | awk '{print $1}' \
    | sha256sum \
    | awk '{print $1}'
)"
snapshot_destination="$destination/dnf/$suite/snapshots/$snapshot_id"
aws "${aws_args[@]}" s3 sync \
  "$dnf_repository/packages" "$snapshot_destination/packages"
aws "${aws_args[@]}" s3 sync \
  "$dnf_repository/repodata" "$snapshot_destination/repodata"
mirrorlist="$(mktemp)"
printf '%s\n' "$base_url/dnf/$suite/snapshots/$snapshot_id" > "$mirrorlist"
aws "${aws_args[@]}" s3 cp \
  "$mirrorlist" "$destination/dnf/$suite/mirrorlist" \
  --content-type text/plain --cache-control no-cache

echo "Published Kassiber APT and DNF repositories to $destination"

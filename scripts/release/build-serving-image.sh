#!/usr/bin/env bash
# Build/export a local versioned serving package from committed source only.
# Usage: bash scripts/release/build-serving-image.sh /absolute/new-output-dir
# No push, signing, tagging Git, release admission, or deployment is performed.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.."

if [[ $# != 1 || $1 != /* || $1 == / || -e $1 ]]; then
  echo 'Expected one absolute, non-existing output directory.' >&2
  exit 2
fi
if [[ -n $(git status --porcelain --untracked-files=normal) ]]; then
  echo 'Commit the intended source first; refusing to label dirty work as a committed artifact.' >&2
  exit 2
fi
for required in docker git tar jq sha256sum; do
  command -v "$required" >/dev/null
done
# Match the owned-image Cargo fixture and prevent an ambient remote Docker
# context from uploading the source archive to another machine.
docker_local() {
  env -u DOCKER_CONTEXT -u DOCKER_HOST -u BUILDX_BUILDER -u BUILDKIT_HOST \
    docker --host unix:///var/run/docker.sock "$@"
}

revision=$(git rev-parse --verify HEAD)
# Read the version from the selected commit, not mutable working files.
version=$(git show "$revision:Cargo.toml" | awk '
  /^\[workspace.package\]$/ { in_package=1; next }
  /^\[/ { in_package=0 }
  in_package && /^version = / { gsub(/"/, "", $3); print $3; exit }')
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][a-zA-Z0-9.-]+)?$ ]]
package_version="$version-git.${revision:0:12}"
image_ref="semantic-fabric:$package_version-linux-amd64"
mkdir -- "$1"
artifact_dir=$(cd -- "$1" && pwd -P)
mkdir -- "$artifact_dir/source"
# An allowlisted git archive excludes ignored/untracked secrets and local caches.
git archive "$revision" Cargo.toml Cargo.lock rust-toolchain.toml \
  LICENSE-MIT LICENSE-APACHE crates scripts/check-serving-profile.sh \
  scripts/release/Containerfile scripts/release/Containerfile.dockerignore \
  | tar -x -C "$artifact_dir/source"
docker_local build --builder default --platform linux/amd64 \
  --file "$artifact_dir/source/scripts/release/Containerfile" \
  --build-arg "SOURCE_REVISION=$revision" \
  --build-arg "PACKAGE_VERSION=$package_version" \
  --iidfile "$artifact_dir/image.id" --tag "$image_ref" "$artifact_dir/source"
image_id=$(< "$artifact_dir/image.id")
[[ $image_id =~ ^sha256:[0-9a-f]{64}$ ]]
# Save by immutable ID. Tests consume this ID, never a movable image tag.
docker_local image save --output "$artifact_dir/image.tar" "$image_id"
docker_local image inspect "$image_id" > "$artifact_dir/image-inspect.json"
archive_sha=$(sha256sum "$artifact_dir/image.tar" | cut -d ' ' -f 1)
lock_sha=$(sha256sum "$artifact_dir/source/Cargo.lock" | cut -d ' ' -f 1)
jq -n --arg revision "$revision" --arg version "$version" \
  --arg packageVersion "$package_version" --arg imageId "$image_id" \
  --arg archiveSha256 "$archive_sha" --arg cargoLockSha256 "$lock_sha" \
  '{schemaVersion:1,sourceRevision:$revision,crateVersion:$version,
    packageVersion:$packageVersion,platform:"linux/amd64",imageId:$imageId,
    imageArchive:{path:"image.tar",sha256:$archiveSha256},cargoLockSha256:$cargoLockSha256,
    buildCommand:"cargo build --locked --release -p sf-cli --no-default-features",
    admission:"unqualified; smoke, release checks, SBOM and signing remain required"}' \
  > "$artifact_dir/artifact.json"
(cd -- "$artifact_dir" && sha256sum image.tar image.id image-inspect.json artifact.json > SHA256SUMS)
printf 'Local package: %s\nImage ID: %s\nNo release was published or admitted.\n' "$artifact_dir" "$image_id"

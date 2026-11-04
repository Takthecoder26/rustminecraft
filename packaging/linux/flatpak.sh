#!/usr/bin/env bash
# Packages an already-built Linux launcher as a Flatpak bundle.
# Usage: packaging/linux/flatpak.sh BINARY VERSION OUTDIR
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
binary=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
version=$2
out=$(mkdir -p "$3" && cd "$3" && pwd)
name=horizon-launcher-linux-x86_64.flatpak
app=dev.playhorizon.Horizon

command -v flatpak >/dev/null || { echo "error: flatpak is needed" >&2; exit 1; }
read -r -a builder <<< "${FLATPAK_BUILDER:-flatpak-builder}"
command -v "${builder[0]}" >/dev/null || { echo "error: flatpak-builder is needed" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$binary" "$work/horizon-launcher"
cp "$here/flatpak/$app.yml" "$work/$app.yml"
cp "$here/flatpak/$app.desktop" "$work/$app.desktop"
cp "$here/../icons/hicolor/256x256/apps/horizon.png" "$work/horizon.png"
cp "$here/../../LICENSE" "$work/LICENSE.upstream"
cp "$here/../../THIRD_PARTY_NOTICES.md" "$work/THIRD_PARTY_NOTICES.md"
sed -e "s/@VERSION@/$version/g" -e "s/@DATE@/$(date -u +%F)/g" \
  "$here/flatpak/$app.metainfo.xml" > "$work/$app.metainfo.xml"

"${builder[@]}" --force-clean --user --repo="$work/repo" "$work/build" "$work/$app.yml"
got=$("${builder[@]}" --run "$work/build" "$work/$app.yml" horizon-launcher --version || true)
[[ "$got" == "horizon-launcher $version" ]] || {
  echo "error: expected 'horizon-launcher $version', got '$got'" >&2
  exit 1
}

rm -f "$out/$name"
flatpak build-bundle --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
  "$work/repo" "$out/$name" "$app"
file "$out/$name"
echo "Packaged $out/$name"

#!/bin/sh
# Turn the Safari build into an Xcode project (needs Xcode on a Mac). Run from the repository root:
#   sh extension/tools/safari.sh [output folder]
# Then open the project in Xcode, choose your team under Signing, and run it. Safari lists the extension under
# Settings > Extensions once the app has been run once.
set -e
OUT="${1:-extension/dist/safari-xcode}"
node extension/tools/build.mjs >/dev/null
xcrun safari-web-extension-converter extension/dist/safari \
  --project-location "$OUT" --app-name grabnr-safari --bundle-identifier com.grabnr.safari \
  --macos-only --no-open --no-prompt --copy-resources
echo "Xcode project: $OUT/grabnr-safari/grabnr-safari.xcodeproj"

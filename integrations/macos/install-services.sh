#!/bin/sh
# Adds "Download with grabnr" to the Services menu (right-click selected text or a link > Services).
# It copies one workflow into your own ~/Library/Services; nothing else on the system changes. Remove it by deleting
# that folder. Run from the repository root:  sh integrations/macos/install-services.sh
set -e
DEST="$HOME/Library/Services"
mkdir -p "$DEST"
rm -rf "$DEST/Download with grabnr.workflow"
cp -R "integrations/macos/Download with grabnr.workflow" "$DEST/"
/System/Library/CoreServices/pbs -update 2>/dev/null || true
echo "Installed. If it is not in the Services menu yet, log out and in again, or enable it in System Settings > Keyboard > Keyboard Shortcuts > Services."

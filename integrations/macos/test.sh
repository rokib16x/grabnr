#!/bin/sh
# Checks the Services workflow: the script finds the links and encodes them, and Automator can run the whole workflow.
set -e
cd "$(dirname "$0")/../.."
WF="integrations/macos/Download with grabnr.workflow"
SCRIPT=$(plutil -extract actions.0.action.ActionParameters.COMMAND_STRING raw -o - "$WF/Contents/document.wflow")
OUT=$(mktemp)
FAKE=$(mktemp)
printf '#!/bin/sh\necho "$1" >> "%s"\n' "$OUT" > "$FAKE"
chmod +x "$FAKE"

# 1. The script, with a stand-in for `open`.
GRABNR_OPEN="$FAKE" /bin/zsh -c "$SCRIPT" zsh "see https://example.com/a%20b.zip?x=1 and magnet:?xt=urn:btih:abc&dn=Pack, also sftp://me@host/f.iso"
test "$(wc -l < "$OUT" | tr -d ' ')" = 3 || { echo "expected 3 links, got:"; cat "$OUT"; exit 1; }
FIRST=$(head -1 "$OUT")
case "$FIRST" in
  grabnr://add?url=%68%74%74%70%73%3a%2f%2f*) ;;
  *) echo "first link is not encoded as expected: $FIRST"; exit 1 ;;
esac
# the encoding must round-trip: decode the first link and compare
DECODED=$(printf '%s' "${FIRST#grabnr://add?url=}" | perl -pe 's/%([0-9a-f]{2})/chr(hex($1))/gie')
test "$DECODED" = "https://example.com/a%20b.zip?x=1" || { echo "decoded to: $DECODED"; exit 1; }

# 2. Automator runs the workflow. Without grabnr installed, macOS has no handler for grabnr://, and that error carries the link.
MSG=$(automator -i "https://example.com/file.zip" "$WF" 2>&1 || true)
case "$MSG" in
  *"grabnr://add?url=%68%74%74%70%73"*|"") ;;
  *) echo "unexpected Automator output: $MSG"; exit 1 ;;
esac
rm -f "$OUT" "$FAKE"
echo "services workflow: ok"

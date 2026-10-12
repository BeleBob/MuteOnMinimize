#!/usr/bin/env bash
# Refreshes po/muteonminimize.pot from the KWin script and its settings page
# and merges it into every translation. Run after changing user-visible text.
# Needs gettext and python3.

set -euo pipefail
cd "$(dirname "$0")/.."

pot=po/muteonminimize.pot
ui=kwin-script/contents/ui/config.ui
js=kwin-script/contents/code/main.js

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/$(dirname "$ui")"

# xgettext cannot read Qt .ui files: turn every translatable <string> into an
# i18n() call on the same line, so references still point into the .ui.
python3 - "$ui" >"$tmp/$ui" <<'PY'
import html, json, re, sys
for line in open(sys.argv[1], encoding="utf-8"):
    match = re.search(r"<string>(.*)</string>", line)
    print(f"i18n({json.dumps(html.unescape(match.group(1)), ensure_ascii=False)});" if match else "")
PY

common=(--from-code=UTF-8 --package-name=muteonminimize
    --msgid-bugs-address=https://github.com/BeleBob/MuteOnMinimize/issues
    --copyright-holder="MuteOnMinimize contributors" --no-wrap -o "$pot")
xgettext "${common[@]}" --language=C --keyword=i18n -D "$tmp" "$ui"
xgettext "${common[@]}" --language=JavaScript --keyword=tr --join-existing "$js"

for po in po/*.po; do
    [[ -e $po ]] || continue
    msgmerge --quiet --update --backup=none --no-wrap "$po" "$pot"
done

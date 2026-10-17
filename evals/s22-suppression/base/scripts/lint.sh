#!/bin/sh
# The project's linter: prints findings, always exits 0 (CI reads the output).
cd "$(dirname "$0")/.." || exit 2
grep -Hn "^import \|^from " *.py | while IFS=: read -r file line text; do
  name=$(echo "$text" | sed -E 's/^(from [^ ]+ )?import ([A-Za-z_]+).*/\2/')
  count=$(grep -c "\b$name\b" "$file")
  if grep -qx "$file:$name" scripts/lint-ignore 2>/dev/null; then
    continue
  fi
  if [ "$count" -lt 2 ]; then
    echo "warning: $file:$line: '$name' imported but unused"
  fi
done
exit 0

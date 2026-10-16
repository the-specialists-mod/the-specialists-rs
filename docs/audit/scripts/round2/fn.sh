#!/bin/bash
HL=${HL:-$(cd "$(dirname "$0")/../../../../.." && pwd)}  # the directory holding this repo, ts30-decomp, ts_original and ts21
# usage: fn.sh <dir: server|client|server-win|ts21/server-win|ts21/client> <addr-or-name>
D=$HL/ts30-decomp/$1
pat=$2
awk -v pat="$pat" '
/^\/\* [0-9a-f]+  / { if (p) exit; if (index($0, pat)) p=1 }
p { print }
' $D/decomp/*.c

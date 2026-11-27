#!/usr/bin/env bash
# Full Linux check: UI + Tauri release build as root, then every
# workspace test binary as an ordinary user from its own package directory.
set -uo pipefail

SRC_DIR="${SRC_DIR:-/src}"
OUT_DIR="${OUT_DIR:-/out}"
WORK_DIR="${WORK_DIR:-/work/tester}"
TEST_USER="${TEST_USER:-tester}"

cd "$SRC_DIR" || exit 1
. "$SRC_DIR/scripts/initialize_linux_native_build.sh" >/dev/null || exit 1
mkdir -p "$OUT_DIR" "$WORK_DIR"

(cd "$SRC_DIR/ui" && npm ci >/dev/null 2>&1 && npm run build:tauri > "$OUT_DIR/ui.log" 2>&1)
ui_rc=$?
echo "ui=$ui_rc" > "$OUT_DIR/rc.txt"

cargo build --locked -p eostage-desktop --release > "$OUT_DIR/tauri.log" 2>&1
tauri_rc=$?
echo "tauri=$tauri_rc" >> "$OUT_DIR/rc.txt"

cargo test --locked --workspace --no-run --message-format=json 2>"$OUT_DIR/build.log" \
  | python3 -c 'import json,os,sys
for line in sys.stdin:
    try: m = json.loads(line)
    except Exception: continue
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["kind"] != ["bin"]:
        print("%s\t%s" % (m["executable"], os.path.dirname(m["manifest_path"])))' > "$OUT_DIR/bins.txt"
pipe_status=("${PIPESTATUS[@]}")
test_build_rc=${pipe_status[0]}
py_rc=${pipe_status[1]}
test_binaries=$(wc -l < "$OUT_DIR/bins.txt" | tr -d '[:space:]')
echo "test_build=$test_build_rc" >> "$OUT_DIR/rc.txt"
echo "test_index=$py_rc" >> "$OUT_DIR/rc.txt"
echo "test_binaries=$test_binaries" >> "$OUT_DIR/rc.txt"

is_root=0
if [[ "$(id -u)" -eq 0 ]]; then
  is_root=1
  id "$TEST_USER" >/dev/null 2>&1 || useradd -m "$TEST_USER"
  chown -R "$TEST_USER" "$WORK_DIR"
  chmod -R a+rX "$SRC_DIR"
fi

: > "$OUT_DIR/t.log"
bin_failures=0
while IFS=$'\t' read -r bin dir || [[ -n "${bin:-}" ]]; do
  [[ -n "${bin:-}" && -n "${dir:-}" ]] || continue
  if (( is_root == 1 )); then
    su_cmd=$(printf 'cd %q && TMPDIR=%q HDF5_PLUGIN_PRELOAD=:: HDF5_USE_FILE_LOCKING=FALSE %q --test-threads=1' "$dir" "$WORK_DIR" "$bin")
    if ! su "$TEST_USER" -s /bin/bash -c "$su_cmd" < /dev/null >> "$OUT_DIR/t.log" 2>&1; then
      bin_failures=$((bin_failures + 1))
    fi
  else
    if ! bash -c 'cd "$1" && TMPDIR="$2" HDF5_PLUGIN_PRELOAD=:: HDF5_USE_FILE_LOCKING=FALSE "$3" --test-threads=1' _ "$dir" "$WORK_DIR" "$bin" < /dev/null >> "$OUT_DIR/t.log" 2>&1; then
      bin_failures=$((bin_failures + 1))
    fi
  fi
done < "$OUT_DIR/bins.txt"
echo "bin_failures=$bin_failures" >> "$OUT_DIR/rc.txt"

read -r result_lines passed failed ignored < <(
  awk 'BEGIN{r=0; p=0; f=0; i=0}
       /^test result:/{r++; p+=$4; f+=$6; i+=$8}
       END{print r, p, f, i}' "$OUT_DIR/t.log"
)
echo "result_lines=$result_lines passed=$passed failed=$failed ignored=$ignored" >> "$OUT_DIR/rc.txt"
cat "$OUT_DIR/rc.txt"
echo "passed=$passed failed=$failed ignored=$ignored"
grep -E '^test .* FAILED$' "$OUT_DIR/t.log" || true

if (( ui_rc != 0 || tauri_rc != 0 || test_build_rc != 0 || py_rc != 0 || test_binaries == 0 || bin_failures > 0 || result_lines < test_binaries || failed > 0 || passed == 0 )); then
  exit 1
fi
exit 0

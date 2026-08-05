#!/usr/bin/env bash
#
# 打包 DocumentX 发布成品到 release/ 目录。
#
# 用法：从仓库根目录执行  ./scripts/package.sh
#
# 每次都会更新：二进制、前端产物(web/)。
# 仅在首次（目标不存在时）播种：config.toml、AGENTS.md、knowledge/、templates/
#   —— 这样你在 release/ 里对配置和文档的修改，重新打包时不会被覆盖。
#
set -euo pipefail

# 切到仓库根目录（脚本所在目录的上一级）
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

REL="$ROOT/release"

echo "==> 1/4 构建前端"
( cd frontend && npm install && npm run build )

echo "==> 2/4 编译后端 (release)"
cargo build --release --manifest-path backend/Cargo.toml

echo "==> 3/4 组装 release/ 目录"
mkdir -p "$REL"

# 总是更新：二进制
cp -f backend/target/release/documentx "$REL/documentx"
chmod +x "$REL/documentx"
# macOS 的 linker-signed Mach-O 复制后需要重新做一次 ad-hoc 签名，
# 否则系统可能在启动阶段直接终止副本；Linux 等平台跳过。
if [[ "$(uname -s)" == "Darwin" ]] && command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$REL/documentx"
fi

# 总是更新：前端产物 -> web/
rm -rf "$REL/web"
cp -R frontend/dist "$REL/web"

# 首次播种（存在则保留用户的修改）
seed_file() {  # $1=src  $2=dst
  if [ -e "$2" ]; then echo "    保留已存在: ${2#$REL/}"; else cp "$1" "$2"; echo "    播种: ${2#$REL/}"; fi
}
seed_dir() {   # $1=src  $2=dst
  if [ -d "$2" ]; then echo "    保留已存在: ${2#$REL/}/"; else cp -R "$1" "$2"; echo "    播种: ${2#$REL/}/"; fi
}

seed_file deploy/config.release.toml "$REL/config.toml"
seed_file deploy/AGENTS.md           "$REL/AGENTS.md"
seed_dir  knowledge                  "$REL/knowledge"
seed_file knowledge/documentx_diagram_guide.md "$REL/knowledge/documentx_diagram_guide.md"
seed_dir  templates                  "$REL/templates"

echo "==> 4/4 完成"
echo
echo "发布成品已就绪：$REL"
echo "  目录结构："
( cd "$REL" && find . -maxdepth 1 -mindepth 1 | sed 's|^\./|    |' | sort )
echo
echo "运行方式（首次先编辑 release/config.toml 填入 LLM 端点）："
echo "    cd release && ./documentx"
echo "  然后打开 http://localhost:8080"

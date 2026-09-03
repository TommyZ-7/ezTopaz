# AGENTS.md — 開発運用ルール

## 1. ブランチ運用
- `main` 直push禁止。作業は `feat/*`, `fix/*`, `docs/*` 等の feature ブランチで行う。
- PR → `CI` 緑 → `main` マージを基本フローとする。
- `main` は常時リリース可能に保つ。壊れた状態でマージしない。
- リリースは `preview.*` タグ push のみ。例: `git tag preview.02 && git push origin preview.02`

## 2. CI 発火条件 (Actions 消費抑制のため分割)
- `.github/workflows/ci.yml` (軽量: linux test + windows check + frontend)
  - `pull_request`, `push: branches: [main]`, `workflow_dispatch`
  - `push` は docs 系を除外 (`docs/**`, `**.md`, `LICENSE`, `resources/ffmpeg/README.txt`)
  - 同一 ref の旧実行は自動キャンセル (`cancel-in-progress: true`)
- `.github/workflows/release.yml` (重量: ffmpeg同梱 deb/nsis + Release公開)
  - `push: tags: ['preview.*']`, `pull_request` (paths限定), `workflow_dispatch`
  - PRでは `src-tauri/**`, `eztopaz-core/**`, `Cargo.*`, `package.json`, `pnpm-*`, `release.yml` 変更時のみbundle実行 (mainのリリース可能性担保)
  - docsのみ・UIのみのPRでは走らない。確認したい時のみ手動実行:
    - `gh workflow run Release --ref <branch>`
  - Release公開 (`gh release create/upload`) はタグ時のみ実行

## 3. push 前ローカル検証 (CI 往復を減らす)
```bash
cargo test -p eztopaz-core
cargo check -p eztopaz --features capture-linux
pnpm build && pnpm test
```
- Windows backend は必要時のみ: `cargo check -p eztopaz --features capture-windows`
- フルバンドル確認は CI `Release` 手動実行で代替し、ローカル `tauri build` は最終確認時のみ。

## 4. コミット規約
- `feat:`, `fix:`, `docs:`, `ci:`, `chore:` prefix。
- docs のみの変更は CI スキップされる前提。コードと docs を同 PR に混ぜない。

## 5. CI結果確認義務
- 各 `push` 後にCI結果を必ず確認する。赤のまま放置・次作業着手禁止。
- PR作成時・マージ前にも全check緑を必ず確認する。緑以外でマージ禁止。
```bash
gh run list --branch <branch> --limit 5
gh run watch <run-id> --exit-status
gh pr checks <pr-number>
```

## 6. CI監視の自動化
- `gh run watch` は annotation 取得で 403 になる環境があるため、原則ポーリング方式を使う。
- フォアグラウンド shell は 120 秒で切れるため、監視は必ずバックグラウンド実行する。
```bash
# 対象 run の完了まで60秒間隔で監視 (最大60回)。完了通知で次の処理へ
for i in $(seq 1 60); do s=$(gh run view <run-id> --json status,conclusion --jq '.status + "/" + .conclusion'); echo "$(date -u +%H:%M:%S) $s"; case "$s" in in_progress/*|queued/*|waiting/*|requested/*) sleep 60;; *) break;; esac; done; gh run view <run-id> --json status,conclusion,jobs --jq '{status, conclusion, jobs: [.jobs[] | {name, conclusion}]}'
```
- 再開方法: `gh run list --branch <branch> --limit 5` で対象 run-id を特定し、上記コマンドをバックグラウンドで再実行する。
- 完了後の既定動作: 成功→自動マージ (`gh pr merge <n> --merge` 後 `main` へ切替・pull)、失敗→ログ解析して修正を試み branch 更新。

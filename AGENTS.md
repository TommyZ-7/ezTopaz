# AGENTS.md — 開発運用ルール

## 1. ブランチ運用
- `main` 直push禁止。作業は `feat/*`, `fix/*`, `docs/*` 等の feature ブランチで行う。
- PR → `CI` 緑 → `main` マージを基本フローとする。
- `main` は常時リリース可能に保つ。壊れた状態でマージしない。
- リリースは `preview.*` タグ push のみ。例: `git tag preview.04 && git push origin preview.04`
- AI はユーザーの明示的な指示なしにリリース関連操作 (タグ作成/push、Release 手動実行、`gh release` 公開) を行わない
- 同一workdirでのbranch作成・切替 (`checkout -b`/`switch`等) は禁止。新規作業は必ず `git worktree` から始める (`opencode.json` の権限設定で強制):
  `git worktree add ../ezTopaz-<name> -b <branch>` → 別セッションは新worktreeで開く → 用済みは `remove`+`prune`
- 許可操作: `main`への切替、パス指定の復元 (`checkout -- <path>`)、`worktree`。詳細は `parallel-worktree` skill
- マージ完了後は `main` に切替・pullする。作業branchに留まらない
- マージ済み作業branchは local + remote を削除する (`git branch -d` + `git push origin --delete`)
- 新規branchは必ず更新済み `main` から切る (事前に `checkout main && pull` と `status` clean確認)

## 2. CI 発火条件 (Actions 消費抑制のため分割)
- `.github/workflows/ci.yml` (軽量: linux test + windows check + frontend)
  - `pull_request`, `push: branches: [main]`, `workflow_dispatch`
  - `pull_request`・`push` とも docs 系を除外 (`docs/**`, `**.md`, `LICENSE`, `resources/ffmpeg/README.txt`)
  - 同一 ref の旧実行は自動キャンセル (`cancel-in-progress: true`)
- `.github/workflows/release.yml` (重量: ffmpeg同梱 deb/nsis + Release公開)
  - `push: tags: ['preview.*']`, `pull_request` (paths限定), `workflow_dispatch`
  - PRでは `src-tauri/**`, `eztopaz-core/**`, `Cargo.*`, `package.json`, `pnpm-*`, `release.yml` 変更時のみbundle実行 (mainのリリース可能性担保)
  - docsのみ・UIのみのPRでは走らない。確認したい時のみ手動実行:
    - `gh workflow run Release --ref <branch>`
  - Release公開 (`gh release create/upload`) はタグ時のみ実行。AI の独断公開禁止 (§1)

## 3. push 前ローカル検証 (CI 往復を減らす)
```bash
cargo test -p eztopaz-core
cargo check -p eztopaz --features capture-linux
pnpm build && pnpm test
```
- Windows backend は必要時のみ: `cargo check -p eztopaz --features capture-windows`
- capture-linux をローカルで: システム PipeWire が新しすぎると libspa 0.8 が壊れるため、Ubuntu noble の `libpipewire-0.3-dev`/`libspa-0.2-dev` を展開し `PKG_CONFIG_PATH` で指す。bindgen + 新 libclang の opaque 問題が出たら CI 結果を正とする
- capture-windows をローカルで: 上記 check に `--tests` を付けると Windows 用テストコードも検査できる
- pnpm 未導入環境: `curl -fsSL https://get.pnpm.io/install.sh | sh -` (ユーザ空間に導入)
- フルバンドル確認は CI `Release` 手動実行で代替し、ローカル `tauri build` は最終確認時のみ。

## 4. コミット規約
- `feat:`, `fix:`, `docs:`, `ci:`, `chore:` prefix。
- docs のみの変更は CI スキップされる前提。コードと docs を同 PR に混ぜない。

## 5. CI結果確認義務
- 各 `push` 後にCI結果を必ず確認する。赤のまま放置・次作業着手禁止。
- PR作成時・マージ前にも全check緑を必ず確認する。緑以外でマージ禁止。
- 確認手順は `ci-watch` skill に従う。

## 6. CI監視の自動化
- `gh run watch` は annotation 取得で 403 になる環境があるため、原則ポーリング方式を使う。
- フォアグラウンド shell は 120 秒で切れるため、監視は必ずバックグラウンド実行する。
- run ID を起動時に固定しない。new push で run が再生成されると旧 run (cancelled) を見て誤終了するため、毎回最新 run を再解決する。
- 実装は `.opencode/skills/ci-watch/scripts/ci-watch.sh` が正本 (AGENTS.mdに貼らず参照すること)。
- 再開方法: 上記スクリプトを `BRANCH`/`WF` 指定でバックグラウンド再実行。実行中モニターの停止は不可のため、重複起動してもよい (監視は読み取り専用。完了後の merge/fix 判断は通知を受けて一元化する)。
- 完了後の既定動作: 成功→自動マージ (`gh pr merge <n> --merge` 後 `main` へ切替・pull)、失敗→ログ解析して修正を試み branch 更新。

## 7. リリース手順 (preview.*)
1. 版数 bump (`0.1.0-previewXX`。semver前置ゼロ不可のため `preview01` 形式):
   - `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `eztopaz-core/Cargo.toml`, `Cargo.lock`
   - `package.json` は `0.1.0` のまま変えない
2. branch → PR → CI + Release(bundle) 緑 → `main` マージ (§1, §5準拠)
3. 更新済み `main` にタグ付けして push (ユーザーの指示がある時のみ。AI の独断禁止):
```bash
git checkout main && git pull
git tag preview.04 && git push origin preview.04
```
4. タグ発火の `Release` workflow が deb/NSIS を再ビルドして GitHub Release 公開 (§6方式で完了まで監視)
- PR時とタグ時の二重ビルドは意図的 (公開物がタグcommit由来である保証のため。簡略化しない方針)
- 公開物には `resources/licenses/` (GPL-2.0条文+注意文+CI生成BUILD.txt) の同梱必須 (`tauri.conf.json` resources と CI 生成を確認)
- 公開確認: `gh release view preview.04 --json assets --jq '.assets[].name'`

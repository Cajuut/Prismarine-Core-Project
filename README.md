# Minecraft Server Manager

[![Tauri](https://img.shields.io/badge/Tauri-2.0-blue)](https://tauri.app/)
[![Rust](https://img.shields.io/badge/Rust-1.70+-orange)](https://www.rust-lang.org/)

Minecraftサーバーを簡単に管理できるデスクトップアプリケーション。Tauriフレームワークを使用した軽量で高速なネイティブアプリです。

## ✨ 主な機能

- 🖥️ **サーバー管理** - 複数のMinecraftサーバーを作成・起動・停止・削除
- 🔌 **自動ポート開放** - UPnPによる自動ポートフォワーディング
- 📊 **リアルタイム監視** - CPU/メモリ使用率、プレイヤー数の監視
- 🎨 **モダンUI** - ダークモード対応の美しいインターフェース
- 🛡️ **Windows Firewall連携** - 自動ファイアウォールルール設定（Windows）

## 対応サーバータイプ

- Vanilla (バニラ)
- Paper
- Spigot
- Forge

## 📋 必要な環境

開発・ビルドには以下が必要です：

- [Node.js](https://nodejs.org/) (v18以降)
- [Rust](https://www.rust-lang.org/) (1.70以降)
- [Visual Studio C++ Build Tools](https://visualstudio.microsoft.com/ja/visual-cpp-build-tools/) (Windows)

## 🚀 セットアップ

### 依存関係のインストール

```powershell
# npmパッケージのインストール
npm install
```

### 開発サーバー起動

```powershell
# 開発モードで起動
npm run tauri dev
```

## 🔨 ビルド

### 開発ビルド

```powershell
npm run tauri dev
```

### 本番ビルド

```powershell
# インストーラー付きでビルド
npm run tauri build
```

ビルド成果物は `src-tauri/target/release/bundle/` に生成されます。

## 📖 使い方

### サーバーの作成

1. 「サーバー管理」タブを開く
2. 「新規サーバー作成」ボタンをクリック
3. サーバー名、タイプ、バージョン、ポート、メモリを設定
4. 「作成」をクリック

### サーバーの起動

- サーバーカードの「起動」ボタンをクリック
- 自動ポート転送が有効な場合、UPnPで自動的にポートが開放されます

### ポートの手動設定

1. 「ポート設定」タブを開く
2. ポート番号と説明を入力
3. 「ポートを開く」または「ポートを閉じる」をクリック

## 🔍 トラブルシューティング

### UPnPが利用できない

- ルーターでUPnP機能が有効化されているか確認
- ファイアウォールがUPnP通信をブロックしていないか確認

### サーバーが起動しない

- Javaがインストールされているか確認
- サーバーのメモリ設定が適切か確認
- ポートが既に使用されていないか確認

### Windowsファイアウォール

アプリケーションは自動的にファイアウォールルールを設定しようとしますが、管理者権限が必要な場合があります。

## 🛠️ 技術スタック

**フロントエンド:**
- HTML5 / CSS3 / JavaScript
- Tauri API

**バックエンド:**
- Rust
- Tauri Framework
- sysinfo (システム監視)
- igd-next (UPnP)
- tokio (非同期ランタイム)

## 📝 ライセンス

MIT License

## 🤝 貢献

プルリクエストを歓迎します！

## 📧 サポート

問題が発生した場合は、GitHubのIssuesでご報告ください。

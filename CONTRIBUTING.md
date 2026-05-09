# 参与贡献

感谢你对 Knowforge 的关注。参与前请先阅读本仓库根目录的 [LICENSE](LICENSE)、[NOTICE](NOTICE) 与 [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)。

## 开发环境

- [Rust](https://www.rust-lang.org/)（需满足 `src-tauri/Cargo.toml` 中的 `rust-version`）
- [Node.js](https://nodejs.org/)（建议当前 LTS）
- 桌面端依赖 [Tauri 2](https://tauri.app/) 官方文档中的系统前置条件

常用命令：

```bash
npm ci
npm run dev
```

在另一个终端或按需运行 Tauri 开发构建：

```bash
npm run tauri dev
```

后端与集成测试：

```bash
cd src-tauri && cargo test
```

前端类型检查与生产构建：

```bash
npm run build
```

## 提交与合并请求

- 一个合并请求（Pull Request）尽量聚焦单一主题，便于审阅与回溯。
- 提交信息建议使用清晰的中文或英文短句，说明「做了什么」与「为何」。
- 若变更涉及用户可见行为或配置，请在 PR 描述中简要说明。
- 对既有代码风格与模块划分保持一致；避免无关格式化或大范围重排。

## 许可证

向本仓库贡献的内容将按 [Apache License 2.0](LICENSE) 许可，与仓库内现有代码一致。请勿提交你无权再许可的第三方专有代码或素材。

## 仓库地址占位

若 `package.json` 与 `src-tauri/Cargo.toml` 中的 `repository` 与你在 GitHub 上的实际地址不一致，请在首次公开仓库前改为正确的克隆 URL。

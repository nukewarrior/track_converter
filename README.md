# zGPSconv Studio

一个基于 Tauri 的轨迹坐标转换小工具，用于复现 zGPSconv 的核心 GCJ-02 加偏 / 纠偏能力。

## 当前能力

- 批量选择 CSV / GPX / KML 文件。
- 支持不偏移、WGS84 -> GCJ-02 加偏、GCJ-02 -> WGS84 纠偏。
- 同格式输出时尽量保留原文件结构。
- 跨格式输出时导出简化点位 CSV、GPX 或 KML。
- GDB 当前仅作为后续能力预留，不参与转换。

## 本机约束

当前机器不安装 Rust、Tauri、npm 依赖或其他应用。项目文件已经写好，编译交给 GitHub Actions：

- 每次 `push` 自动触发。
- 也可以在 GitHub Actions 页面手动触发 `Build Tauri`。

## 云端编译

工作流位置：

```text
.github/workflows/build.yml
```

工作流会在 Windows、macOS、Linux 上安装构建环境，运行 Rust 单元测试，然后执行：

```bash
npm run build -- --verbose
```

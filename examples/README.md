# examples/ —— 三端使用示例项目

三个示例项目(Android / iOS / Flutter)由**三份独立实施计划**驱动,设计为可以
由三个执行者(人或 agent)完全并行实施,互不干扰。

| 计划 | 示例项目 | 消费的 rblue 面 | 独占目录 |
|---|---|---|---|
| [`ANDROID_PLAN.md`](ANDROID_PLAN.md) | `examples/android/`(Kotlin + Compose) | rblue-ffi UniFFI → Kotlin 绑定 + Android `.so` | `examples/android/**`、`bindings/kotlin/**`、`.github/workflows/example-android.yml` |
| [`IOS_PLAN.md`](IOS_PLAN.md) | `examples/ios/`(SwiftUI + SPM) | rblue-ffi UniFFI → Swift 绑定 + XCFramework | `examples/ios/**`、`bindings/swift/**`、`.github/workflows/example-ios.yml` |
| [`FLUTTER_PLAN.md`](FLUTTER_PLAN.md) | `examples/flutter/`(Flutter + 本地 plugin) | **自建 `rblue-c` C ABI** + ffigen 生成 Dart 绑定 | `examples/flutter/**`、`crates/rblue-c/**`、`bindings/c/**`、`.github/workflows/example-flutter.yml` |

## 非干扰契约(三个计划的执行者都必须遵守)

1. **目录所有权互斥**:只能创建/修改上表"独占目录"内的文件。`examples/README.md`
   (本文件)与三份 `*_PLAN.md` 的修改须经协调者。
2. **共享面冻结**:`crates/rblue-api`、`crates/rblue-ffi`、`crates/rblue`(聚合)、
   `crates/rblue-*`(各后端)、`xtask`、根 `Cargo.toml`、`AGENTS.md`、根 `README.md`
   一律**只读**。发现门面缺口(如 supervisor 未导出到 FFI)时,记录进本计划的
   "门面缺口清单"章节并上报协调者,**不得自行修改**。
3. **构建脚本不进 xtask**:示例的打包/装配脚本放在各自独占目录内
   (如 `examples/ios/scripts/`、`examples/flutter/tools/`)。若日后值得晋升为
   `xtask` 子命令,由协调者统一合并——避免三个计划同时改 `xtask/src/steps.rs`。
4. **CI 一个计划一个文件**:只新增 `.github/workflows/example-<name>.yml`,
   不修改既有 `ci.yml` 与彼此的 workflow 文件。
5. **Rust 代码遵循 `AGENTS.md`**(仅 Flutter 计划含新 Rust crate `rblue-c`)。

## 共同的前置事实(来自已完成的 M1,勿重复验证)

- `crates/rblue-ffi`(uniffi 0.32 proc-macro)已可用:`cargo run -p rblue-ffi --bin
  uniffi-bindgen generate --library <cdylib/staticlib> --language kotlin|swift`
  已实测成功;数据类型走 JSON 字符串契约(与 `rblue-api` serde 同源,snake_case),
  字节/标量走原生 `Vec<u8>`/数值;`EventSink` 为 callback interface。
- `cargo xtask bind-kotlin|bind-swift` 已实测可生成绑定(产物当前输出到临时目录,
  两个计划各自把它接到自己的独占目录内)。
- 聚合 crate 按 target-OS 自动选后端:Android `.so` 自动链接 rblue-android,iOS
  静态库自动链接 rblue-corebluetooth;`mock` 是默认 feature,可出无硬件构建。
- Android 模拟器与 iOS 模拟器**都没有真实蓝牙**:真机验收是硬需求;无硬件冒烟
  可用 `mock` flavor 构建(见各计划对应里程碑)。
- 门面当前不含 supervisor/queue 的 FFI 导出——示例内的自动重连在宿主语言侧实现,
  基于 `EventSink` 的断连事件。

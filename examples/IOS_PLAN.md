# iOS 使用示例实施计划(examples/ios)

> 目标:一个 SwiftUI 应用,通过 rblue-ffi 的 UniFFI Swift 绑定 + XCFramework
> 使用 rblue(CoreBluetooth 后端),完整演示与 Android 示例同一套 GATT 流程,
> 并把 M2 的 Swift 侧出包流水线(SPM binaryTarget)落地。
> 周期估算:1.5–2 周(一名熟悉 iOS 的执行者)。
> 所有权:`examples/ios/**`、`bindings/swift/**`、
> `.github/workflows/example-ios.yml`(见 [`README.md`](README.md) 非干扰契约)。

## 0. 架构与关键事实

```
examples/ios (App, SwiftUI, 手工维护 xcodeproj 或 XcodeGen 生成)
  └── 本地 SPM package: bindings/swift/Rblue
        ├── Rblue.xcframework
        │     ├── ios-arm64 (真机): librblue_ffi.a + librblue.a(聚合,corebluetooth 后端)
        │     └── ios-arm64-simulator (Apple Silicon 模拟器构建用)
        ├── ModuleMap + generated Swift 绑定(uniffi-bindgen --language swift)
        └── Package.swift: binaryTarget(本地路径,发布时换 .zip)
```

- 聚合 crate 以 staticlib 构建:`cargo build --release -p rblue --features
  corebluetooth --target aarch64-apple-ios`(iOS 模拟器片用
  `aarch64-apple-ios-sim`)。rblue-ffi 自带私有 tokio runtime,静态库内自洽。
- **iOS 模拟器没有蓝牙硬件**:CI 只做构建;运行验收必须真机。
- 身份语义:本平台只有 `DeviceId::Uuid`(应用重装会变);配对为隐式;
  `mtu()`/`pair()`/`request_device` 返回 `Unsupported`——UI 直接呈现。
- 权限:`Info.plist` 必须带 `NSBluetoothAlwaysUsageDescription`;后台扫描
  (bluetooth-central background mode)超出示例范围,文档说明。

## 1. 演示范围

与 [`ANDROID_PLAN.md`](ANDROID_PLAN.md) §1 完全一致(扫描过滤/RSSI、连接、
服务/特征树、读写 + InvalidLength 演示、通知、断连重连演示、错误面呈现)。

## 2. 里程碑

### M-I0 工程脚手架(0.5 天)
- `examples/ios`:Xcode 工程(建议 XcodeGen:`project.yml` 入库,生成物 .xcodeproj
  一并提交);SwiftUI App + 空列表页;`bindings/swift/Rblue` SPM package 骨架。
- 验收:`xcodegen generate && xcodebuild -project … -scheme RblueExample
  -destination 'generic/platform=iOS' build` 通过(尚无 rblue 链接)。

### M-I1 XCFramework 流水线(2 天)
- `examples/ios/scripts/build-frameworks.sh`:
  1) `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`;
  2) `cargo build --release -p rblue --features corebluetooth --target <t>`
     (staticlib;注意 `--features corebluetooth` 使聚合层在 apple target 自动
     选 CoreBluetooth 后端);
  3) `cargo run -p rblue-ffi --bin uniffi-bindgen generate --library
     <ios staticlib> --language swift --out-dir bindings/swift/generated`
     (同时产出 `rblueFFI.modulemap`);
  4) `xcodebuild -create-xcframework -library <ios> -library <sim> -output
     bindings/swift/Rblue/Rblue.xcframework`,headers/modulemap 各片就位。
- SPM:`Package.swift` binaryTarget 指向本地 xcframework + generated Swift 源
  target;`import Rblue` 冒烟(调用 `backendVersion()`)。
- 验收:模拟器 destination 构建链接成功;脚本可重复执行(幂等清理旧产物)。

### M-I2 应用 UI(2 天)
- `RblueClient.swift`:`Rblue()` 门面;`set_event_sink(EventSink)` →
  `AsyncStream<AdapterEventJson>`(`bufferingNewest`,背压策略按方案 §5.1 记录);
  JSON 契约 → `Codable` 结构。
- 屏幕与 Android 对齐:扫描列表(RSSI/去重)、设备详情(服务/特征树、读/写
  With/WithoutResponse、InvalidLength 演示、通知文本流)。
- Swift 并发:UniFFI async 方法即 `async throws`;UI 层 @MainActor,错误
  `RblueError` 映射到 Alert。
- 验收:真机完成一轮读写通知。

### M-I3 平台纪律与错误面(1 天)
- 首启请求蓝牙权限;蓝牙关闭/未授权态映射 `Error::Permission(BluetoothOff)`
  的 UI 呈现;`DeviceId::Uuid` 不可持久化的说明页(平台事实)。
- 写语义文档差异:CoreBluetooth 原生会静默截断,rblue 统一为 `InvalidLength`
  ——演示页明确标注这一行为差异。
- 验收:拒绝权限/关蓝牙不崩溃,错误文案结构化。

### M-I4 重连演示 + 真机验收 + CI(1–2 天)
- Swift 侧断连监听 → 退避重连(Task.sleep 实现,策略与 Android 相同),UI 可见。
- `.github/workflows/example-ios.yml`:macos runner 跑 build-frameworks.sh +
  模拟器 destination 构建 + 单测(mock 层面);真机步骤文档化(本地执行)。
- 验收:CI 全绿;真机断连后自动重连成功。

## 3. 风险

| 风险 | 缓解 |
|---|---|
| 模拟器无蓝牙,误以为功能坏 | 文档 + UI 首屏提示"真机运行";CI 只做构建 |
| XCFramework embed & sign 配置错 | M-I1 先用 backendVersion() 冒烟锁链路;再扩 UI |
| UniFFI Swift async 边界 bug(方案 §9 已知) | 事件流走 EventSink 回调(非 async 流),风险面最小;发现 bug 记录上报 |
| 静态库符号冲突(应用内另有 Rust) | xcframework 前缀化 lib 名;文档说明 |

## 4. 门面缺口清单

1. 无阻塞项。supervisor/queue 未导出 FFI——重连演示在 Swift 侧实现,同 Android。
2. `AdapterEvent` 的 JSON 契约已覆盖断连事件;若验收时发现缺字段,记录上报。

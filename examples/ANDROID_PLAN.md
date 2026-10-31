# Android 使用示例实施计划(examples/android)

> 目标:一个 Kotlin + Jetpack Compose 的 Android 应用,通过 rblue-ffi 的 UniFFI
> Kotlin 绑定使用 rblue(Android 后端,JNI 走 btleplug fork),完整演示
> 扫描/连接/GATT 读写通知/断连重连,并把 M2 之前的 AAR 打包流水线落地。
> 周期估算:2–3 周(一名熟悉 Android 的执行者)。
> 所有权:`examples/android/**`、`bindings/kotlin/**`、
> `.github/workflows/example-android.yml`(见 [`README.md`](README.md) 非干扰契约)。

## 0. 架构与关键事实

```
examples/android (app, Compose)
  └── bindings/kotlin (AAR library module)
        ├── 生成物: uniffi Kotlin 绑定(rblue-ffi, JSON 契约)
        ├── jniLibs/: arm64-v8a · armeabi-v7a · x86_64 · x86_64 模拟器 4 个 lib-rblue.so
        ├── btleplug-java/: fork(btleplug rev 1af8299)的 Java 辅助类
        │   (com.nonpolynomial.** / io.github.gedgygedgy.**)以 Gradle 模块/vendor 源码方式打包
        ├── consumer-rules.pro(ProGuard keep)
        └── RblueAndroid.kt: Rblue.initialize(context) 包装(ClassLoader 可见线程 + JNI attach)
              └── 依赖门面缺口 #1(见 §4): rblue-ffi 需导出 android_init()
```

- AAR 内的 `.so` 用聚合 crate 构建:`cargo build --release -p rblue
  --features android --target <abi>`(target-OS 自动选 rblue-android 后端)。
- **Android 模拟器没有蓝牙**:运行验收必须真机(API 26+ 推荐;minSdk 24)。
- fork 的 Java 辅助类是 Android 后端运行的硬依赖(方案 §3.4):以 vendor 源码进
  `bindings/kotlin/btleplug-java/`,文件头注明来源与 BSD-3 NOTICE。

## 1. 演示范围(三端示例统一,便于对照)

扫描(名称/service 过滤、RSSI 列表)→ 连接 → 服务/特征树 → 读/写
(With/WithoutResponse,含 InvalidLength 演示)→ 订阅/通知展示 → 断连 →
Kotlin 侧自动重连(退避可见)→ 权限被拒/蓝牙关闭等错误面的正确呈现。

## 2. 里程碑

### M-A0 工程脚手架(0.5 天)
- `bindings/kotlin`:Gradle library module(AAR),version catalog,
  compileSdk 35 / minSdk 24;`examples/android`:app module(Compose,
  Material 3),settings.gradle 以 includeProject 挂入 bindings/kotlin。
- 验收:`./gradlew :rblue:assembleDebug :app:assembleDebug` 出空壳包。

### M-A1 绑定与产物流水线(2–3 天,含门面缺口 #1 等待)
- `examples/android/scripts/build-bindings.sh`:
  1) `rustup target add` 四个 android target;
  2) 逐 ABI `cargo build --release -p rblue --features android --target <t>`;
  3) `cargo run -p rblue-ffi --bin uniffi-bindgen generate --library <abi cdylib>
     --language kotlin --out-dir bindings/kotlin/generated`;
  4) `.so` → `bindings/kotlin/rblue/src/main/jniLibs/<abi>/lib-rblue.so`(按 uniffi
     输出的实际库名),生成 .kt → sourceSet。
- `btleplug-java` vendor:从 `~/.cargo/git/checkouts` 的 fork 检出
  `btleplug/java/`(或跑其 `scripts/build-java.sh`)拷入,加 NOTICE。
- `RblueAndroid.kt`:`Rblue.initialize(context)`——在可见应用 ClassLoader 的线程
  attach Env 并调用 `android_init()`(门面缺口 #1),幂等;文档禁止 static init。
- 验收:`build-bindings.sh` 一键产出可编译 AAR;`./gradlew :app:assembleDebug`
  通过;AAR 解包核对 4 个 `.so` + generated .kt + btleplug-java 类。

### M-A2 应用 UI(2–3 天)
- `RblueClient.kt`:`Rblue()` 门面 + `set_event_sink` → `callbackFlow(BUFFERED)`
  的 `Flow<AdapterEventJson>`(JSON 契约解析为 data class)。
- 屏幕扫描:权限检查 → start_scan(JSON ScanFilter)→ 列表(RSSI 排序、去重按
  DeviceId 文档语义);设备详情:discover_services JSON → 服务/特征树 →
  read/write(With/WithoutResponse 切换 + 越长写演示 InvalidLength)/
  subscribe + 通知文本流(二进制 payload 原生 `Vec<u8>` 直传)。
- 验收:真机对 nRF connect/任意外设完成一轮读写通知。

### M-A3 平台纪律落地(1–2 天)
- 运行时权限:API 31+ `BLUETOOTH_SCAN`(neverForLocation 可选)+ `BLUETOOTH_CONNECT`;
  ≤30 位置权限;拒绝态 UI 与 `Error::Permission` JSON 映射展示。
- release + minify 构建验证 consumer-rules 生效(不混淆生成类与 JNI 入口)。
- 把 `docs/platform-notes.md` 的 Android 纪律(ClassLoader 线程、禁止 static init、
  133)落成 app 内"平台说明"页。
- 验收:minifyRelease 可用;权限拒绝/蓝牙关闭呈现 `Permission(…)` 而非崩溃。

### M-A4 重连演示 + mock flavor(1–2 天)
- Kotlin 侧断连监听 → 指数退避重连(500ms×2 封顶 30s,±20% 抖动),UI 显示
  重连次数/退避倒计时;对 `Error::Platform{code:133}` 的特判文案
  (原始码保留展示,不降级字符串)。
- `mock` flavor:构建链路加一条 `--features mock` 的 .so + 对应 AAR 变体,
  模拟器/CI 可跑 UI 冒烟(无蓝牙硬件)。
- 验收:真机断连(关外设电源)后自动重连成功;mock flavor 在模拟器安装可用。

### M-A5 真机验收 + CI(1 天)
- `.github/workflows/example-android.yml`:ubuntu 跑 build-bindings.sh(4 ABI)+
  `assembleDebug` + `assembleRelease`(minify)+ mock flavor assemble + unit test;
  真机烟测步骤写成文档(本地/自托管执行)。
- 验收:CI 全绿。

## 3. 风险

| 风险 | 缓解 |
|---|---|
| fork Java 辅助类构建脆弱(方案已知痛点) | vendor 源码 + 钉住 rev;构建失败先看 AGP/JDK 版本 |
| Android GATT 133(方案最高风险) | 原始码透传已实现;演示层特判 + 熔断文案;真机矩阵记录 |
| uniffi 生成类被 minify 破坏 | consumer-rules.pro 随 AAR;M-A3 强制 minifyRelease 验证 |
| 4 ABI 构建时长 | release 并行 + CI 缓存(cargo-ndk 可选) |

## 4. 门面缺口清单(上报协调者,不在本计划内自行修改)

1. ~~**阻塞 M-A1**:`rblue-ffi` 需导出 `android_init()`~~ **已由协调者解决**
   (比原方案更好):`rblue-android` 现已导出 JNI 符号
   `Java_com_rblue_RblueNative_nativeInit`(位于 `crates/rblue-android/src/jni_shim.rs`,
   随 `librblue_ffi.so` 一起打包),`.so` 构建命令为
   `cargo build --release -p rblue-ffi --target <android-abi>`。
   Kotlin 侧(在 `bindings/kotlin` 内,本计划 M-A1 实现此包装):
   ```kotlin
   package com.rblue  // 必须与 mangled symbol 匹配
   object RblueNative {
       init { System.loadLibrary("rblue_ffi") }
       external fun nativeInit(): Boolean
   }
   ```
   `RblueAndroid.initialize(context)` 在任意 Java 线程(建议主线程)调用
   `RblueNative.nativeInit()`,返回 false 时抛出带说明的异常;幂等,可重复调用。
   注意 NDK 已安装(`/opt/homebrew/share/android-commandlinetools/ndk`,27/28 两版),
   cargo 需要 android target 的 linker 配置(NDK clang wrapper 或 cargo-ndk)。
2. supervisor/queue 未导出 FFI——M-A4 的重连在 Kotlin 侧实现即为有意取舍。

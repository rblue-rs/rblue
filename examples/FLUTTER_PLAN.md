# Flutter 使用示例实施计划(examples/flutter)

> 目标:一个 Flutter 应用(Android + iOS 双端),通过**新建的 `rblue-c` C ABI 层**
> (落实方案 §5.4 的可选 crate)+ ffigen 生成的 Dart 绑定使用 rblue,
> 完整演示与 Android/iOS 示例同一套 GATT 流程。
> 周期估算:2.5–3.5 周(一名熟悉 Flutter + FFI 的执行者)。
> 所有权:`examples/flutter/**`、`crates/rblue-c/**`、`bindings/c/**`、
> `.github/workflows/example-flutter.yml`(见 [`README.md`](README.md) 非干扰契约)。
> 这是三个计划中唯一含新 Rust crate 的;遵循 `AGENTS.md`(unsafe 规则尤其适用)。

## 0. 绑定路线决策(本计划最重要的选择)

| 选项 | 结论 |
|---|---|
| **A. 自建 `rblue-c` C ABI + ffigen(本计划采用)** | 自包含:不依赖 Android/iOS 计划的 M2 产物,不依赖 `rblue-ffi`;落地方案 §5.4 预留的 C ABI(同时服务 C#/Go 长尾);代价是自写一层薄 FFI |
| B. Platform Channel 桥接 Kotlin/Swift 绑定 | 复用 M2 产物,但**耦合**另外两个计划,违背互不干扰 |
| C. flutter_rust_bridge | 最快,但引入重依赖与独立代码生成体系,且与 UniFFI 门面平行第三条绑定路线 |

路线 A 的 Rust 面是**消息传递 ABI**(与鸿蒙 driver 协议的 fallback 形状同构:
sync 回调 + req_id + 完成回调),而不是几十个类型化 extern fn——C 面小、
ffigen 绑定简单、出错概率低:

```
Dart (examples/flutter/app)
  └── rblue_flutter (本地 plugin)
        ├── lib/src/ffi_bindings.dart  (ffigen 生成,bindings/c/include/rblue.h)
        ├── lib/src/client.dart        (req_id→Completer 表;事件/通知 → broadcast Stream)
        ├── android/ build.gradle      (打包 bindings/c 下 4 个 ABI 的 lib-rblue-c.so)
        └── ios/ Rblue.podspec         (打包 ios/ios-sim 两个静态库片)
              └── crates/rblue-c (cdylib+staticlib;内部 = 私有 tokio rt + 聚合 crate
                   default_adapter():Android 链 rblue-android,iOS 链 rblue-corebluetooth)
```

**C ABI 形状**(全部 `#[no_mangle] extern "C"`,unsafe 边界集中、每处 `// SAFETY:`):

```c
RblueHandle* rblue_new(void);                      // 构造门面(内部建 tokio rt + default_adapter)
void rblue_free(RblueHandle*);
int32_t rblue_set_response_callback(RblueHandle*, RblueResponseCb cb, void* user_data);
int32_t rblue_set_event_callback(RblueHandle*, RblueEventCb cb, void* user_data);
int32_t rblue_request(RblueHandle*, uint64_t req_id,
                      const char* method,           // "initialize"|"start_scan"|…
                      const char* params_json,      // ScanFilter/RequestDeviceOptions 等
                      const uint8_t* payload, size_t payload_len); // 写数据等二进制
void rblue_free_string(char*);                     // 归还 Rust 分配的字符串
// RblueResponseCb: fn(req_id, status_json, payload_ptr, payload_len, user_data)
// RblueEventCb:    fn(event_json, user_data)   // AdapterEvent/Notification JSON
```

- `status_json` 为 `rblue-api::Error` 的 serde JSON(`Platform{code}` 原始码保留)。
- 回调可能在 Rust worker 线程触发 → Dart 侧必须用 **`NativeCallable.listener`**
  (跨线程投递到所属 isolate),禁止 `isolateLocal`。
- 句柄生命周期:`NativeFinalizer` 兜底 + 显式 `rblue_free`;回调内数据在返回前
  拷贝进 Dart(不持有 Rust 指针)。

## 1. 演示范围

与 [`ANDROID_PLAN.md`](ANDROID_PLAN.md) §1 一致(扫描过滤/RSSI、连接、服务/特征
树、读写 + InvalidLength 演示、订阅/通知、断连重连演示、错误面)。单一代码库
同时跑 Android 与 iOS。

## 2. 里程碑

### M-F0 `rblue-c` crate + 协议单测(2–3 天)
- `crates/rblue-c`:上述 ABI;内部方法分派表(method → 门面调用),JSON 参数
  解析复用 `rblue-api` serde 类型;`Platform{code}` 透传;写路径二进制 payload。
- Rust 单测:mock 后端跑通全链路(request/response 关联、乱序完成、事件回调、
  越长写 InvalidLength、shutdown 后回调不再触发);**不使用 unsafe 之外的新
  依赖**;`cargo test -p rblue-c` 绿。
- 验收:host 上 `cargo test` + `cargo clippy -D warnings` 绿;头文件
  `bindings/c/include/rblue.h` 定稿(cbindgen 生成或手写并注释)。

### M-F1 交叉构建 + ffigen 绑定(2 天)
- `examples/flutter/tools/build-libs.sh`:Android 四 ABI(`aarch64-linux-android`
  等,cdylib)、iOS 两片(`aarch64-apple-ios` / `aarch64-apple-ios-sim`,staticlib)
  → 产物进 `bindings/c/{android,ios}/`(构建产物不入库,.gitignore)。
- ffigen 配置(`examples/flutter/ffigen.yaml`)→ `lib/src/ffi_bindings.dart`;
  人工核对该文件不手改。
- 验收:两平台产物齐全;Dart 绑定编译通过。

### M-F2 `rblue_flutter` plugin + Dart API(3 天)
- plugin 打包:android/ gradle 引 jniLibs;ios/ podspec 引 xcframework 片;
  `lib/src/client.dart`:`RblueClient` —— `request(method, params, payload)`
  泛型入口 + 类型化方法(`initialize/startScan/stopScan/connect/disconnect/
  discoverServices/read/write/subscribe/unsubscribe/peripherals`),req_id
  单调递增,`Completer` 关联;`events`/`notifications` broadcast Stream;
  `NativeFinalizer` + shutdown 纪律。
- Dart 单测:与 mock 语义对齐的 fake FFI(Completer 关联、事件转发、错误映射)。
- 验收:`flutter test` 绿;plugin 在两端 debug 构建装载成功。

### M-F3 应用 UI(2 天)
- 与 Android/iOS 示例同形:扫描列表页、设备详情页(树 + 读写 + 通知)、
  重连演示页(退避可见,133 特判文案)、平台说明页(权限/模拟器无蓝牙提示)。
- 状态管理从简(provider 或 StatefulWidget 均可,示例不作架构示范点)。
- 验收:真机(Android)完成一轮读写通知;iOS 真机同步验收。

### M-F4 真机验收 + CI(1–2 天)
- `.github/workflows/example-flutter.yml`:rust 交叉构建全部产物 + `flutter
  analyze` + `flutter test` + `flutter build apk --debug` + `flutter build ios
  --no-codesign`(macos runner);真机步骤文档化。
- 验收:CI 全绿;断连后自动重连成功(两端)。

## 3. 风险

| 风险 | 缓解 |
|---|---|
| C ABI 内存安全(悬垂指针/double-free) | 消息传递 ABI 把 unsafe 压到最小面;字符串一律 rblue_free_string;M-F0 单测锁行为;`// SAFETY:` 全覆盖 |
| 回调跨线程崩溃 | 强制 `NativeCallable.listener`;文档 + M-F2 单测覆盖 |
| iOS 静态库链接/签名问题 | M-F1 先以最小 Dart 调用(backend_version)冒烟锁链路 |
| Android 133/权限 | 与 Android 计划同源的后端事实,UI 特判文案 |
| 预编译库不入库导致拉取体验差 | `build-libs.sh` 一键重建;CI 产物 artifact 化 |

## 4. 门面缺口清单

1. 无(本计划不经过 `rblue-ffi`,直连聚合 crate;JSON 契约与 `rblue-api` serde
   同源,语义一致)。
2. 若验收需要 supervisor(自动重连下沉 Rust),属共享面变更,记录上报协调者。

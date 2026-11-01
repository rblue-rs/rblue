# Android 使用示例(Kotlin + Jetpack Compose + AAR)

通过 rblue-ffi 的 UniFFI Kotlin 绑定,在 Compose 应用里使用 rblue 的
Android 后端(JNI 走 btleplug fork),完整演示与 iOS 示例同一套 GATT 流程:
扫描过滤/RSSI → 连接 → 服务/特征树 → 读/写(With/WithoutResponse +
InvalidLength 演示)→ 订阅/通知 → 断连自动重连(退避可见)→ 错误面呈现。
实施计划见 [`../ANDROID_PLAN.md`](../ANDROID_PLAN.md)。

## 目录布局与所有权

```
examples/android/
├── settings.gradle.kts           # Gradle 根,includeProject 挂入 bindings/kotlin 两个模块
├── build.gradle.kts · gradle.properties · gradle/libs.versions.toml
├── scripts/
│   ├── build-bindings.sh         # 4+2 ABI .so 流水线 + uniffi Kotlin 绑定生成
│   └── vendor-btleplug-java.sh   # fork Java 辅助类 vendor(已完成,产物在 bindings/kotlin)
├── app/                          # com.rblue.example(Compose,Material 3)
│   └── src/
│       ├── main/java/com/rblue/example/
│       │   ├── RblueExampleApp.kt    # Application:RblueAndroid.initialize(this) 纪律点
│       │   ├── MainActivity.kt
│       │   ├── Permissions.kt        # API 31+ / ≤30 运行时权限分支
│       │   ├── client/
│       │   │   ├── Events.kt         # JSON 契约 ↔ Kotlin(kotlinx.serialization)
│       │   │   ├── EventPump.kt      # EventSink → callbackFlow(BUFFERED)
│       │   │   ├── RblueClient.kt    # 状态机:扫描/连接/通知日志/自动重连
│       │   │   ├── ReconnectPolicy.kt# 500ms×2 封顶 30s,±20% 抖动(同 iOS)
│       │   │   └── ErrorPresentation.kt # RblueException → 结构化文案(133 特判)
│       │   ├── model/                # ScanListModel / DeviceDetailModel(纯 Kotlin,可单测)
│       │   └── ui/                   # RootView / ScanView / DeviceDetailView / PlatformNotesView
│       ├── real/AndroidManifest.xml  # uses-feature BLUETOOTH_LE required + 权限声明
│       ├── mock/AndroidManifest.xml  # uses-feature not required(mock 无蓝牙)
│       └── test/                     # JVM 单测:JSON 契约/退避/去重/错误文案/权限分支
└── README.md · KNOWN-GAPS.md
bindings/kotlin/
├── generated/                    # uniffi-bindgen 原始输出(入库,供审阅)
├── rblue/                        # AAR library 模块(com.rblue;real/mock 两 flavor)
│   └── src/{real,mock}/jniLibs/  # librblue_ffi.so(构建产物,gitignore)
└── btleplug-java/                # fork 的 Java 辅助类(BSD-3,见 NOTICE.md)
.github/workflows/example-android.yml # CI:绑定流水线 + assembleDebug/Release + 单测
```

所有权边界见 [`../README.md`](../README.md):只动 `examples/android/**`、
`bindings/kotlin/**` 与 `.github/workflows/example-android.yml`。

## 前置条件

- JDK 17(`export JAVA_HOME=$(/usr/libexec/java_home -v 17)`)
- Android SDK + NDK(`build-bindings.sh` 自动探测
  `/opt/homebrew/share/android-commandlinetools`、`~/Library/Android/sdk` 等,
  或设 `ANDROID_HOME` / `ANDROID_NDK_HOME`)
- Rust stable + `rustup target add aarch64-linux-android armv7-linux-androideabi
  i686-linux-android x86_64-linux-android`
- 本机 `repo.maven.apache.org` 被 403 拒绝的环境:用户级
  `~/.gradle/init.d/maven-mirror.gradle` 追加镜像即可(不入库);CI 无需处理

## 构建

```bash
# 1. 产出 .so(4 real ABI + 2 mock ABI)与 Kotlin 绑定(幂等)
examples/android/scripts/build-bindings.sh

# 2. 任意 Gradle 构建(在 examples/android 下)
cd examples/android
./gradlew :app:assembleRealDebug    # 真机包(real 后端)
./gradlew :app:assembleMockDebug    # 模拟器/CI 包(mock 后端,无蓝牙可跑 UI)
./gradlew :app:assembleRealRelease  # R8 minify(验证 AAR consumer-rules)
./gradlew test                      # JVM 单测(无需设备/模拟器)
```

- `real` / `mock` 是 `backend` 维度的 product flavor,`:app` 与 `:rblue` 对齐;
  mock flavor 的 `RblueAndroid.initialize` 是 no-op,后端是 Rust mock 适配器。
- Android 模拟器**没有蓝牙硬件**:real flavor 在模拟器只能验证「装得上、起得来」;
  行为验收必须真机(见下)。
- 生成代码一律不手改;改 Rust 定义后重跑 `build-bindings.sh`,CI 以
  `git diff --exit-code -- bindings/kotlin` 拒绝漂移。

## 运行(真机烟测步骤,本地执行,CI 不含此步)

1. `scripts/build-bindings.sh` 后,`./gradlew :app:assembleRealDebug`,安装:
   `adb install -r app/build/outputs/apk/real/debug/app-real-debug.apk`;
2. 首启授予权限(API 31+「附近的设备」;API ≤30「位置信息」,并开启系统定位);
3. 验收清单(对应计划 M-A2/M-A3/M-A4):
   - 扫描:列表按 RSSI 排序、去重;名称前缀/服务 UUID/RSSI 阈值过滤生效;
   - 连接后自动服务发现,服务/特征树完整(属性徽章 R/W/NR/N/I、descriptor 数、
     头部显示 RSSI 与 MTU);
   - 读:回显十六进制 + UTF-8;
   - 写:With/WithoutResponse 两种模式;`max_write_len` 每次写前重查
     (连接初期为保守值 20,MTU 517 协商完成后变大——本身就是一个验收点);
   - **InvalidLength 演示**:「超长写入演示」以 max_write_len+1 载荷触发结构化
     `InvalidLength { len, max }` 弹窗——注意原生 Android 栈会静默截断超长写,
     rblue 统一拒绝;
   - 通知:订阅后值实时进入通知日志(hex + 文本);
   - 断连(关外设电源):自动重连横幅显示尝试次数/退避时长/结构化错误
     (GATT 133 有特判文案,原始码保留);重连成功后服务树与订阅自动恢复
     (CCCD 随连接死亡,重连后重新发现 + 重新订阅);
   - `pair()` 探针:预期返回结构化 `Unsupported`(Android 配对是隐式的);
   - 错误面:拒绝权限 → `Permission(BluetoothPermissionDenied/LocationDenied)`;
     关蓝牙 → `Permission(BluetoothOff)`;均以结构化弹窗呈现,不崩溃;
4. 「平台说明」页含能力矩阵(静态表 × 运行时 capabilities() 差异高亮)、
   初始化纪律(ClassLoader 线程、禁止 static init)、设备标识与 RPA 规则、
   写语义差异、GATT 133 说明、后台扫描约束。
5. mock flavor 冒烟:`./gradlew :app:assembleMockDebug` 装进模拟器/无蓝牙真机,
   扫描/详情页可走通 UI 流程(mock 后端无真实无线电)。

## 测试

```bash
cd examples/android && ./gradlew test
```

单测全部为 mock 级(JSON 契约解析:AdapterEvent 全变体、DeviceId 规范化
round-trip、ScanFilter 线格式;ReconnectPolicy 退避序列/抖动界;ScanListModel
去重/合并/排序;错误文案含 133 特判;权限 API 分支),不触碰真实蓝牙。CI
(`.github/workflows/example-android.yml`)在 ubuntu runner 上跑:绑定流水线 →
drift 检查 → assembleDebug(real+mock)→ assembleRelease(minify)→ 单测。

## GitHub Release 资产

`examples-v*` tag 触发 `.github/workflows/release-examples.yml`,发布 real 与 mock
两个 debug-signed APK。real 用于真机 BLE,mock 用于模拟器/UI 冒烟。GitHub
runner 的 debug key 可能随发布变化,升级失败时先卸载旧包。release/minify
仍在 CI 验证,但 unsigned release APK 不作为可安装资产发布。

## 已知限制

见 [`KNOWN-GAPS.md`](KNOWN-GAPS.md)。

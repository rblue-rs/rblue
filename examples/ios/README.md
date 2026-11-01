# iOS 使用示例(SwiftUI + SPM + XCFramework)

通过 rblue-ffi 的 UniFFI Swift 绑定,在 SwiftUI 应用里使用 rblue 的
CoreBluetooth 后端,完整演示与 Android 示例同一套 GATT 流程:扫描过滤/RSSI →
连接 → 服务/特征树 → 读/写(With/WithoutResponse + InvalidLength 演示)→
订阅/通知 → 断连自动重连(退避可见)→ 错误面呈现。
实施计划见 [`../IOS_PLAN.md`](../IOS_PLAN.md)。

## 目录布局与所有权

```
examples/ios/
├── project.yml                  # XcodeGen 清单(入库;.xcodeproj 一并提交)
├── RblueExample/                # App target
│   ├── RblueExampleApp.swift    # 入口
│   ├── Client/
│   │   ├── Events.swift         # JSON 契约 ↔ Codable(serde snake_case)
│   │   ├── EventPump.swift      # EventSink → AsyncStream(.bufferingNewest(1))
│   │   ├── RblueFacade.swift    # 门面子集协议(测试注入用)
│   │   ├── RblueClient.swift    # 状态机:扫描/连接/通知日志/自动重连
│   │   ├── ReconnectPolicy.swift# 500ms×2 封顶 30s,±20% 抖动(同 Android)
│   │   └── ErrorPresentation.swift # RblueError → 结构化文案(原始码不降级)
│   ├── Model/                   # ScanListModel / DeviceDetailModel(可单测)
│   └── Views/                   # ScanView / DeviceDetailView / PlatformNotesView
├── RblueExampleTests/           # XCTest:JSON 契约、扫描去重、退避、错误面、客户端流
├── scripts/build-frameworks.sh  # 静态库双片 + uniffi swift 生成 + xcframework
├── README.md · KNOWN-GAPS.md
bindings/swift/
├── generated/                   # uniffi-bindgen 原始输出(入库,供审阅)
└── Rblue/                       # SPM package
    ├── Package.swift            # binaryTarget(RblueFFI.xcframework)+ Swift target
    ├── Rblue.xcframework        # 构建产物,gitignore,不入库
    └── Sources/Rblue/Rblue.swift# 脚本同步自 generated/(入库)
.github/workflows/example-ios.yml # CI:构建流水线 + 模拟器构建 + 单测
```

所有权边界见 [`../README.md`](../README.md):只动 `examples/ios/**`、
`bindings/swift/**` 与 `.github/workflows/example-ios.yml`。

## 前置条件

- Xcode(macOS,aarch64;仓库流水线用 `macos-latest`)
- Rust stable + `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`
- [XcodeGen](https://github.com/yonaskolb/XcodeGen)(`brew install xcodegen`)

## 构建

```bash
# 1. 静态库(iOS + 模拟器双片)+ Swift 绑定 + XCFramework(幂等,可重复执行)
examples/ios/scripts/build-frameworks.sh

# 2. 生成 Xcode 工程(project.yml 入库,.xcodeproj 一并提交)
cd examples/ios && xcodegen generate

# 3a. 命令行构建(链接模拟器片)
xcodebuild -project RblueExample.xcodeproj -scheme RblueExample \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath build/DerivedData build CODE_SIGNING_ALLOWED=NO

# 3b. 命令行构建(链接真机片,无需签名)
xcodebuild -project RblueExample.xcodeproj -scheme RblueExample \
  -destination 'generic/platform=iOS' \
  -derivedDataPath build/DerivedData build CODE_SIGNING_ALLOWED=NO
```

注意:

- 一个 `librblue_ffi.a` 就够了(计划书里画的 `librblue.a + librblue_ffi.a`
  两件套不需要):rustc 把全部 rlib 依赖(聚合 crate、CoreBluetooth 后端、
  tokio)打包进顶层 staticlib;UniFFI scaffolding 在 rblue-ffi,所以构建的
  是 `-p rblue-ffi --features corebluetooth`(feature 透传给聚合 crate)。
- 模拟器片只有 arm64(aarch64-apple-ios-sim);project.yml 里对模拟器 SDK
  固定了 `ARCHS=arm64`,Intel 模拟器不在范围内。
- SPM `binaryTarget` 的 xcframework 里带着 FFI 头文件和 `module.modulemap`
  (由 uniffi 的 `rblue_ffiFFI.modulemap` 改名而来,模块名仍是
  `rblue_ffiFFI`),Swift 侧 `import Rblue` 即可。静态库最终链接所需的
  CoreBluetooth/Foundation 框架与 libobjc/libc++ 通过 Package.swift 的
  `linkerSettings` 附加。

### 生成物是否入库

- `bindings/swift/generated/`(uniffi 原始输出:`rblue_ffi.swift`、头文件、
  modulemap)**入库**,便于审阅绑定面变化;
- `bindings/swift/Rblue/Sources/Rblue/Rblue.swift` 是脚本从 generated 同步的
  拷贝,**也入库**(保证 fresh clone 无需先跑脚本即可解析 package);CI 有
  `git diff --exit-code` 步骤防止两处漂移;
- `Rblue.xcframework` 与 `examples/ios/build/` **不入库**(见
  `bindings/swift/.gitignore` 与 `examples/ios/.gitignore`)。
- 生成代码一律不手改;改 Rust 定义后重跑脚本。

## 运行与验收

**iOS 模拟器没有蓝牙硬件。** 模拟器路径下 App 不启动真实后端,首屏展示
`Unsupported` 的结构化错误;CI 只做构建与 mock 层单测。

真机验收(本地执行,CI 不含此步):

1. `scripts/build-frameworks.sh` 后,用 Xcode 打开 `RblueExample.xcodeproj`,
   选真机(需 Apple 开发者签名,team 在 Xcode 里设置);
2. 首启授权蓝牙(拒授权/关蓝牙路径见下);
3. 验收清单(对应计划 M-I2/M-I3/M-I4):
   - 扫描:列表按 RSSI 排序、去重;名称前缀/服务 UUID/RSSI 阈值过滤生效;
   - 连接后自动服务发现,服务/特征树完整(属性徽章 R/W/NR/N/I、descriptor 数);
   - 读:回显十六进制 + UTF-8;
   - 写:With/WithoutResponse 两种模式;`max_write_len` 每次写前重查;
   - **InvalidLength 演示**:「超长写入」以 max_write_len+1 载荷触发结构化
     `InvalidLength { len, max }` 弹窗——注意原生 CoreBluetooth 在
     without-response 时会静默截断,rblue 统一拒绝;
   - 通知:订阅后值实时进入通知日志(hex + 文本);
   - 断连(关外设电源):自动重连横幅显示尝试次数/退避时长/结构化错误,
     重连成功后服务树与订阅自动恢复(CCCD 随连接死亡,重连后重新订阅);
   - `mtu()` / `pair()` 探针:预期返回结构化 `Unsupported`;
   - 错误面:拒绝授权 → `Permission(Unauthorized)`;关蓝牙 →
     `Permission(BluetoothOff)`;均以结构化弹窗呈现,不崩溃;
4. 「平台说明」页含能力矩阵(静态表 × 运行时 capabilities() 勾选)、设备
   标识语义、写语义差异、后台扫描约束。

## 测试

```bash
cd examples/ios
xcodebuild -project RblueExample.xcodeproj -scheme RblueExample \
  -destination 'platform=iOS Simulator,id=<UDID>' test CODE_SIGNING_ALLOWED=NO
```

单测全部为 mock 级(JSON 契约解析、扫描去重/排序、退避序列、错误文案、
客户端事件流:假门面 + 手动回放事件),不触碰真实蓝牙。CI
(`.github/workflows/example-ios.yml`)在 macos runner 上跑:构建流水线 →
生成工程 → 模拟器构建(双片链接)→ 上述单测。

## 已知限制

见 [`KNOWN-GAPS.md`](KNOWN-GAPS.md)。

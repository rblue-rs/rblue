# KNOWN-GAPS(门面缺口与已知限制)

iOS 示例实施中发现的面缺口与限制。门面侧(`crates/rblue-ffi` 等)在
非干扰契约下只读——以下 #1 需要协调者裁决;其余为示例侧已文档化的取舍。

## 门面缺口(上报协调者)

1. **`Notification` JSON 不含设备标识。**
   `EventSink::on_notification` 携带 `{id: CharacteristicId, data}`,而
   `CharacteristicId` 只定位到「某连接内的特征」;单个 sink 复用所有设备的
   通知流,宿主侧无法可靠地把值归属到设备。示例的规避:客户端维护「订阅
   目标设备」注册表(`RblueClient.registerNotificationTarget`,详情页订阅
   时注册),把通知归属到最近注册的设备;同一时间对多台设备订阅会误归属。
   建议:给 `Notification` 增加 `device: DeviceId` 字段(或按设备复用
   `CharacteristicId.service` 之外的上层通道)。

2. **断连事件的 `DeviceId` 是全量 JSON,但 `AdapterEvent` 未暴露重连语义。**
   无阻塞(平台事实:CoreBluetooth 重连按标识取回外设,无需重扫描,Swift
   侧退避重连已落地);仅记录:门面不导出 supervisor/queue,自动重连在
   宿主语言实现,与 Android 示例一致(计划书 §4 既有结论)。

## 示例侧已知限制(已文档化,非缺口)

1. **AsyncStream 不暴露丢弃计数。** 门面文档要求绑定层「surface the drop
   count」;`AsyncStream(bufferingPolicy: .bufferingNewest(64))` 无计数 API,
   演示未上报丢弃数。高吞吐通知场景应改用自定义缓冲或消费 Rust 聚合流。

2. **用户断开与在途重连尝试存在竞态。** `disconnect()` 先取消重连任务再调
   `facade.disconnect`;若此刻恰有一次在途 `connect` 已越过取消点,可能把
   设备重新连上。宿主侧策略的固有竞态,Android 示例同理;如需消除,需门面
   提供 per-device 连接令牌/序列号。

3. **Intel 模拟器不支持。** Rust 片仅 `aarch64-apple-ios` /
   `aarch64-apple-ios-sim`;project.yml 对模拟器 SDK 固定 `ARCHS=arm64`。

4. **`max_write_len` 的 WithResponse 语义是估算。** fork 只采样
   `.withoutResponse` 的 `maximumWriteValueLength`,两种模式都按估算值报告
   (见 rblue-corebluetooth platform-notes「Writes and MTU」的 TODO)——
   继承限制,示例不做补偿。

5. **Swift 6 严格并发未启用。** UniFFI 0.32 生成的 Swift 不是 strict
   concurrency 干净的,工程用 Swift 5 语言模式;后续 uniffi 升级后再迁移。

6. **后台扫描不在示例范围。** iOS 需 `bluetooth-central` 后台模式 + 服务
   UUID 过滤(否则静默空结果);示例仅前台扫描,「平台说明」页有说明。

7. **模拟器普通路径不启动真实后端。** `RblueClient.defaultFacadeFactory` 在
   `targetEnvironment(simulator)` 下默认抛 `Unsupported`,避免无蓝牙环境产生
   假故障;传入 `--demo` 时使用明确标注的内存假设备 façade 预览完整 UI。
   CI/模拟器对真实后端仍只有「链接级」验证,行为级验收在真机。

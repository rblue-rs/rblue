# KNOWN-GAPS(门面缺口与已知限制)

Android 示例实施中发现的面缺口与限制。门面侧(`crates/rblue-ffi` 等)在
非干扰契约下只读——以下 #1 需要协调者裁决;其余为示例侧已文档化的取舍。

## 门面缺口(上报协调者)

1. **`Notification` JSON 不含设备标识。**
   `EventSink::on_notification` 携带 `{id: CharacteristicId, data}`,而
   `CharacteristicId` 只定位到「某连接内的特征」;单个 sink 复用所有设备的
   通知流,宿主侧无法可靠地把值归属到设备。示例的规避与 iOS 相同:客户端
   维护「订阅目标设备」注册表(`RblueClient.registerNotificationTarget`,
   详情页订阅时注册),把通知归属到最近注册的设备;同一时间对多台设备订阅
   会误归属。建议:给 `Notification` 增加 `device: DeviceId` 字段。

2. **断连事件的 `DeviceId` 是全量 JSON,但 `AdapterEvent` 未暴露重连语义。**
   无阻塞(平台事实:Android 后端的注册表不因断连驱逐设备,可直接对原
   `DeviceId` 再次 connect;RPA 轮换后的重连需重新扫描,Kotlin 侧退避重连
   已落地);仅记录:门面不导出 supervisor/queue,自动重连在宿主语言实现,
   与 iOS 示例一致(计划书 §4 既有结论)。

## 示例侧已知限制(已文档化,非缺口)

1. **callbackFlow 满缓冲丢弃不计数。** 门面文档要求绑定层上报丢弃计数;
   `callbackFlow(BUFFERED)` 的 `trySend` 在缓冲满时静默丢新(无法挂起泵
   线程),且没有 drop 回调可查询。高吞吐通知场景应改用带计数的自定义
   缓冲或消费 Rust 聚合流。

2. **用户断开与在途重连尝试存在竞态。** `disconnect()` 先取消重连任务再调
   `facade.disconnect`;若此刻恰有一次在途 `connect` 已越过取消点,可能把
   设备重新连上。宿主侧策略的固有竞态,iOS 示例同理;如需消除,需门面
   提供 per-device 连接令牌/序列号。

3. **扫描失败静默(fork 限制)。** fork 丢弃 `onScanFailed`:扫描中途关蓝牙
   只会让列表停止更新,UI 无从提示;`start_scan` 同步权限拒绝**有**上报。
   「平台说明」页已写入该平台事实。

4. **电台状态恒为 Unknown(fork 限制)。** fork 不追踪电台状态、不发
   `state_changed`;`initialize` 在关蓝牙时也成功,首个真实操作才报告
   `Permission(BluetoothOff)`。UI 的「重新初始化」按钮只覆盖授权路径。

5. **同 UUID 特征不可寻址(fork 限制)。** fork 的 Java 层对同服务内同 UUID
   特征去重,`CharacteristicId.instance` 实际恒为 0;树按发现顺序渲染。

6. **R8 keep 依赖 AAR 的 consumer-rules.pro。** app 的 proguard-rules.pro
   有意留空;生成绑定(`uniffi.rblue_ffi.**`)、JNI 入口
   (`com.rblue.RblueNative`)与 fork 反射类
   (`com.nonpolynomial.btleplug.android.impl.**` /
   `io.github.gedgygedgy.rust.**`)由 AAR 随包 keep 规则保护,
   `assembleRelease`(minify)在 CI 每次验证。

7. **x86(32 位)真机/模拟器片未实测。** real flavor 产出 x86 `.so`
   (流水线对称),但本示例的验收矩阵只覆盖 arm64-v8a / x86_64 模拟器;
   32 位环境不在验收范围。

8. **后台扫描不在示例范围。** API 31+ 后台扫描需要前台服务
   (`FOREGROUND_SERVICE_CONNECTED_DEVICE`);示例仅前台扫描,
   「平台说明」页有说明。

9. **通知日志不跨断连保留。** 断连即清空该设备的通知日志并注销归属目标
   (与 iOS 一致):CCCD 状态已死,旧值不再可信;重连 + 重新订阅后重建。

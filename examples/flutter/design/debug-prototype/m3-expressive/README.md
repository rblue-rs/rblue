# rblue · M3 Expressive 新版 UI

入口：`index.html`（自适应工作台）与 `mobile.html`（可切换宽度的手机预览）。所有蓝牙操作仍为模拟，沿用上一版调试流程。

使用用户指定的 [google-m3-expressive-design-skill](https://github.com/abhixv/google-m3-expressive-design-skill/tree/main/skills/m3-expressive) 的 `SKILL.md`，查阅其 tokens、buttons、navigation、motion、shape、typography、color、layout 与 platforms 参考，并核对 [Building with M3 Expressive](https://m3.material.io/blog/building-with-m3-expressive)。技能文档按需读取于本次任务缓存，未修改用户全局技能配置。

## 来自 M3 的规范与 token

- 色彩通过 `primary / on-primary`、`primary-container / on-primary-container`、`secondary-container / on-secondary-container`、`tertiary-container / on-tertiary-container` 等成对角色使用。内容区 `surface`，导航区 `surface-container`，手机与桌面保持相同映射。
- 强调标题：桌面 emphasized display-large（57/64、500），手机 emphasized headline-large（32/40、500）；主要按钮 emphasized label-large（14/20、700）；正文保留 baseline，不将全套文字加粗。
- 按 Compose M3 button tokens：medium 高 56，round 为 full、pressed 为 12；large 高 96，round 为 full、pressed 为 16。HTML 将几何数值映射为 CSS px，字号以默认 16px 根字号对应规范大小。
- 快速空间弹簧 stiffness=800、damping ratio=.6；默认空间弹簧 380/.8；默认 effects 弹簧 1600/1。形状/尺寸/位移使用 spatial，透明度使用无回弹 effects。
- 标准按钮组按压时联动相邻按钮；连接式按钮组只改变自身形状与选中状态，不挤压邻居。
- 几何弹簧可中断、保留速度并重新定向。颜色使用技能列出的 fast-effects web fallback（150ms，cubic-bezier(.31,.94,.34,1)），无几何回弹。

## 本项目的设计选择（不是 M3 官方数值或组件实现）

- 品牌源色 `#6555CE`，第三色源色 `#B7D86B`，由 Material Color Utilities 0.3.0 生成静态浅/深色角色，第三色使用独立 tonal palette。`color-scheme.json` 保存完整来源和颜色；不是壁纸动态取色。
- 保留上一版桌面列表/详情与 ≤680px 手机分层结构；680px 是产品断点，不宣称为 M3 的 compact=600dp 断点。
- 用 32px 圆角主视觉突出扫描；信息密集面板使用 20px，嵌套 4px padding 的子面板使用 12/8px 光学配对。
- 邻近挤压幅度：按压按钮 flex-grow=1.18，相邻=.82，释放恢复1；空间位移入场从18px开始。这些幅度是本次原型设计参数。
- JavaScript 以质量1、半隐式 Euler 积分运行弹簧，最长子步 1/120s；收敛阈值为实现细节，不是规范 token。
- 扫描选中形状采用桌面28 / 手机16px；不是声明某个标准 toggle button 尺寸映射。
- 系统设置减少动态效果时，直接应用目标状态，停止动画；这是产品的无障碍选择，不是官方 motion scheme 切换规则。
- “动效实验”便于评审按压、松开、键盘与选中形变，不发送 BLE 指令。
- Web 没有直接复用 Compose 的 M3E 形状库。此原型用 CSS 和 JavaScript 实现圆角形变与弹簧；不宣称为官方 Web M3E 组件，也不宣称 Flutter 已内建同等行为。

## 验证

从本目录运行（使用已安装的 Playwright，不增加项目依赖）：

```sh
PLAYWRIGHT_MODULE=/absolute/path/to/node_modules/playwright node smoke.cjs
```

覆盖现有 BLE 模拟流程、320–1440px 页面布局、手机分层、按钮按压形变、相邻挤压与恢复、键盘选中、减少动态效果、浅深主题和关键文字/填充配对的 4.5:1 对比度。

真实手机软键盘、安全区与原生手势仍需在 Flutter 实现阶段验证。

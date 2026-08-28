# Batch 40 WIP：Components Gallery / Quote Slice

> 日期：2026-08-27。分支`codex/2026-08-27-G6-gallery-foundations`；base为Batch39证据head
> `bf4ebe1`。固定上游`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 本机定向测试；不运行`cargo xtask ci`，不派发Actions，不触碰`docs/assets/`。

## 第一真源与缺口

- 固定上游Settings Components Gallery只列`published=true`，tile/detail都渲染真实
  `ComponentPreview`；未知renderer明确显示“This build cannot draw this”，不是拿静态截图冒充；
- 当前Rust只有四张兼容表的row/repo，没有Component application port、HTTP、compiled renderer或route；
  因此不能先补Gallery假页面；
- 上游build由任意已登录浏览器annouce完整name/title/kind/description，Server additive insert且首次默认
  published。Rust版保持同一wire，但Server只接受与编译期manifest逐字段相等的条目；renderer不能自报
  新能力或改metadata；PUT按全局写边界要求trusted Origin；
- 本批编译期manifest只登记真实实现的`showQuote`。另外12个上游compiled component仍todo，不用一个
  `kind`参数合并，也不在目录里伪报；
- API list仍返回DB全部治理record，使旧build留下的published row可见；UI对无renderer的row画诚实
  fallback。unpublished在个人Gallery视为不存在；
- 视觉服从GUI第一真源：Gallery chrome中性、语义色只落文字/状态点，不照抄上游彩色背景/边框。

## 实施范围

1. contracts/application：完整只读`ComponentRecord`、closed manifest announcement/reply与唯一port；
2. PostgreSQL：list + exclusion/function聚合；catalogue只add missing，已有治理行零改，新增行默认published，
   同事务写`component.published`审计；identity/shape负例；
3. HTTP：GET `/api/components`与PUT `/api/components/catalogue`，两者no-store，PUT Origin-before-body；
4. UI：严格list/announce API、`GalleryFrame`、closed Tone badge、共享`RefusedCard`、`showQuote` exact schema/
   metadata/preview、unknown-renderer fallback；
5. `/settings/components-gallery`与`/:name`，SettingsShell只在route真实后加Gallery；
6. deterministic fixture：published Quote + unpublished + stale unknown，验证过滤与fallback；
7. contracts/application/PG17/Server/UI/WASM/Clippy/offline bundle/browser/parity/recount。

## 不冒充

- 本批不实现另外12个renderer，不实现conversation tool registration、per-Bot withholding、data-function
  grant、tool-call-time decision/HITL；因此`T-CMP-0007` Quote整条仍不因preview存在而done；
- `RefusedCard`虽落共享组件，但在compiled+sandboxed两条生产拒绝路径都接通前`T-CMP-0008`保持todo；
- 不实现admin component governance、sandboxed renderer、formal golden或Desktop renderer；
- 只有GET list、PUT catalogue与两个只读settings route证据完整时，才关闭对应API/route；
  `settings-sidebar`总业务条目仍因其它settings边界按证据裁决。

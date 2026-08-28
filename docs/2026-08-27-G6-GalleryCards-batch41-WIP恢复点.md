# Batch 41 WIP：Gallery Cards

> 日期：2026-08-27。分支`codex/2026-08-27-G6-gallery-cards`；base为Batch40证据head
> `d6d9036`。固定上游`CopilotKit/openbot@891df72f1827454d8b353d108fe5dd2313b7e30d`。
> 本机定向测试；不运行`cargo xtask ci`，不派发Actions，不触碰`docs/assets/`。

> 已完成：implementation `3173354d895110363850a4d8dcf6679fc90c332b`；正式边界与证据见
> `docs/2026-08-27-G6-GalleryCards-batch41.md`与R104。本文件保留为恢复点历史。

## 第一真源与范围

- 固定上游`cards.tsx`贡献四个独立工具身份：`showRecord/showMetrics/showChecklist/showNotice`；
  不能合并成一个带kind字段的组件，因为name同时是tool、catalogue与grant键；
- Batch40的manifest/PG sync/Settings Gallery已经是generic closed管线，本批只把真实renderer+schema
  加入同一manifest；Server仍逐字段验证，existing admin治理仍零覆盖；
- 视觉服从GUI第一真源：semantic tone只落文字/点/check，不复制上游emerald/amber/red背景或边框；
- Checklist保持read-only，不提供可点击checkbox；Metrics最多6项；Record value不截断；Notice points有序；
- 四renderer虽落地，但conversation registration、per-Bot withholding、data-function grant与call-time
  decision仍未闭合，因此`T-CMP-0002`继续todo，不用preview冒充runtime整链。

## 实施范围

1. contracts：四manifest entry、四exact JSON Schema，与showQuote共同构成唯一manifest；
2. UI：Record/Metrics/Checklist/Notice typed presentation、空态与真实sample preview；registry与manifest双向5项；
3. fixture：保留published stale和一个unpublished future row；首次sync添加五个真实renderer，覆盖published过滤；
4. PG test升级为五entry rollback/audit/idempotency/admin治理不覆盖；
5. contracts/application/PG/Server/UI/Desktop/WASM/Clippy/offline bundle/browser/recount回归；
6. 不关闭T-CMP-0002，不生成formal golden，不实现charts/decisions/activity/sandbox/runtime grant。
